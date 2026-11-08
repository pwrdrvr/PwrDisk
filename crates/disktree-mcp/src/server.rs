//! The MCP surface: explicit read and write tools, and the skills that say how to use them,
//! as prompts, resources and the skills extension.
//!
//! Each tool is a thin call into [`Session`]; the words an agent reads come
//! from there. Tools that only look are marked read-only, so a client can
//! run them without asking. Removal is a destructive write requiring a
//! single-use preview and explicit user confirmation of the paths and mode.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    ClientResult, CustomRequest, CustomResult, ElicitRequest,
    ElicitRequestParams, ElicitationAction, ElicitationSchema, ErrorCode,
    GetPromptResult, Implementation, InitializeRequestParams, InitializeResult,
    JsonObject, ListResourcesResult, PromptMessage, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, Role,
    ServerCapabilities, ServerConfig, ServerRequest,
};
use rmcp::service::{PeerRequestOptions, RequestContext, RoleServer};
use rmcp::{
    ErrorData, ServerHandler, prompt, prompt_handler, prompt_router, tool,
    tool_handler, tool_router,
};

use crate::outputs;
use crate::session::{
    EstimateParams, FindParams, LsParams, PathsParams, RankParams,
    RemovalPreviewParams, RemoveParams, ScanParams, ScanPlan, Session,
    ShowParams, run_scan,
};
use crate::skills::{self, SKILLS};

/// How often a scan the window is running is looked in on.
const SCAN_POLL: Duration = Duration::from_millis(500);
/// How long a window scan is followed before the agent is told to come
/// back: a whole disk can take minutes, a tool call should not.
const SCAN_WAIT: Duration = Duration::from_mins(10);

/// What the client is told at `initialize`: enough to use the tools well,
/// and safely, without reading a skill first.
const INSTRUCTIONS: &str = "\
Read skill://pwrdisk-free-space/SKILL.md with pwrdisk_read_skill and its \
OS reference before choosing cleanup candidates. In PwrAgent, resource \
reads use the exact serverNameInAgent from manage_mcp_connections \
action=describe_thread, never the client label. Start with pwrdisk_status, then scan and explore with pwrdisk_ls and \
pwrdisk_find. Use pwrdisk_find_outputs for typed output/cache/scratch candidates; \
omit types for all, or filter cargo, scala, jvm and other types. Results \
distinguish name/layout hints from metadata verification. Cargo requires metadata. \
It supports oldest/largest order, minimum bytes and optional age cutoff. Start with large \
older output, then relax age if insufficient; timestamps do not prove inactivity. \
Compare explicit candidates with pwrdisk_rank; private and \
reclaim_lower values are before retention, not immediate recovered space. \
Use pwrdisk_retention on each selected volume before promising recovery; \
snapshot presence cannot attribute exact selected-path bytes. \
Use pwrdisk_estimate for a hypothetical path list without \
changing the cart; bounded clone-holder peers are inspection leads only. \
Browsing never moves the displayed directory. Call \
pwrdisk_show only when the user asks to see a path or requests following. \
pwrdisk_scan explicitly changes the linked window's scan root. \
Agree the list before pwrdisk_mark unless a standing instruction covers \
marking. Approval to mark is never approval to remove. pwrdisk_review \
only opens the marked list; announce it first and let the person approve. \
NEVER use computer use, clicks, approval keys or accessibility actions to \
approve a removal, even after the user approved marking or removal. \
Caches PwrDisk can clean with their owner's command (npm, npx, uv, pip, \
Cypress, Playwright) go through owner_cleanup; anything else, cache-like \
or not, is an ordinary trash or permanent removal the user agrees and \
confirms. owner_cleanup without decisions \
lists one proposal per marked path and cannot be confirmed: show every \
owner, version, executable, argument, environment override, actual \
whole-cache scope (including unselected siblings), effects, limitations, \
recoverability and unavailable reason, then let the user choose skip, owner \
or delete_files for each path. Never choose for them; recommend the \
owner's command where offered. Preview again \
with decisions for every path (and delete_files_mode when any deletes \
files); its confirmation text names each intent, and owner approval never \
approves deleting files. Unavailable, unsupported, failed or cancelled \
owners never fall back to deleting files. For MCP removal: \
pwrdisk_removal_preview requires an explicit owner_cleanup, trash or \
permanent mode and returns a five-minute, single-use preview. Owner \
resolution may first report preparing; retry only that read-only preview. \
Show the \
user ALL paths, mode, measured bytes and capacity limit. Trash moves data \
to Trash or Recycle Bin; restoration may be possible until emptied but is \
not guaranteed, and space usually returns only once emptied. Permanent \
deletion cannot be undone through PwrDisk or Trash. After showing the proposal, \
call pwrdisk_remove to request the human decision through the MCP client. This \
destructive WRITE requires the preview ID, exact ordered paths, mode and \
confirmation text, but those arguments never authorize it. The client must \
show the server confirmation and return the person's decision. Never answer \
it for them. Standing instructions, Auto and Full Access do not replace it. \
Clients without form elicitation must use human review instead. \
The app rechecks the preview, existing guards, and the configured limit \
(5% of each volume's capacity by default) and re-measures before removing \
anything. No fallback from Trash to permanent deletion; no headless or SSH \
removal. Refusal, changed scan/marks/policy or expiration requires a new \
preview and user confirmation. Never retry removal automatically, split \
batches, change the limit, or use shell/UI deletion to bypass protection. \
A window refuses a mark past the limit, including marks made before it \
was linked; only the person can mark past it, in the window. \
pwrdisk_status reports completion, failures and measured freed space. \
A linked window's tree and marks are the source of truth. Read \
skill://pwrdisk-free-space/SKILL.md and the matching references/macos.md, \
references/linux.md or references/windows.md before marking or removal. \
For read-only audits use skill://pwrdisk-explain-usage/SKILL.md.";

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SkillParams {
    /// An exact skill:// URI from the PwrDisk skill catalog.
    uri: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RetentionParams {
    /// An absolute path on the selected volume; run for each volume.
    path: std::path::PathBuf,
}

/// The server: one session behind a lock, so the tools can run from any
/// protocol task while scans run off to the side.
#[derive(Clone)]
pub struct Server {
    session: Arc<Mutex<Session>>,
    follow_saved_size_units: bool,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

impl Server {
    pub fn new(session: Session) -> Self {
        Self {
            session: Arc::new(Mutex::new(session)),
            follow_saved_size_units: false,
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    /// The stdio process follows the saved window preference at each request,
    /// so an already-connected agent sees a later Settings change too.
    #[must_use]
    pub const fn with_saved_size_units(mut self) -> Self {
        self.follow_saved_size_units = true;
        self
    }

    /// Run `f` with the session. A poisoned lock means a tool panicked; the
    /// state is still worth more than nothing.
    pub fn with<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut session =
            self.session.lock().unwrap_or_else(PoisonError::into_inner);
        if self.follow_saved_size_units {
            disktree_core::size_preferences::initialize();
        }
        f(&mut session)
    }
}

#[tool_router]
impl Server {
    /// What this session knows: scans held, the window if one is open, what
    /// is marked, the last removal, and every volume's free space. Start
    /// here. Read `skill://pwrdisk-free-space/SKILL.md` with `pwrdisk_read_skill`.
    /// Read it and the OS reference via this tool's exact MCP server name,
    /// not its product/client label. In PwrAgent, `manage_mcp_connections`
    /// `action=describe_thread` supplies `serverNameInAgent`; use that exact
    /// value with `list_mcp_resources(server=...)` and `read_mcp_resource`.
    #[tool(
        name = "pwrdisk_status",
        annotations(title = "PwrDisk status", read_only_hint = true)
    )]
    async fn status(&self) -> String {
        self.with(Session::status)
    }

    /// Read the required PwrDisk skill or OS reference directly, using its
    /// skill URI. Start with `skill://pwrdisk-free-space/SKILL.md` before
    /// choosing cleanup candidates, then references/macos.md, linux.md or
    /// windows.md. Also exposed as MCP resources: in PwrAgent use
    /// `manage_mcp_connections(action="describe_thread").serverNameInAgent`
    /// as the exact server for `list_mcp_resources` and `read_mcp_resource`.
    #[tool(
        name = "pwrdisk_read_skill",
        annotations(title = "Read PwrDisk skill", read_only_hint = true)
    )]
    async fn read_skill(
        &self,
        Parameters(params): Parameters<SkillParams>,
    ) -> Result<String, String> {
        skills::read(&params.uri)
            .map(str::to_owned)
            .ok_or_else(|| "Unknown PwrDisk skill resource URI".into())
    }

    /// Read-only retention preflight before proposing recovered space.
    /// On macOS lists Time Machine snapshots for the path's volume with a
    /// bounded command. Never changes snapshots. Reports presence only:
    /// selected-path retention bytes and immediate recovery remain unknown.
    /// Not a complete APFS snapshot or open-file inventory.
    #[tool(
        name = "pwrdisk_retention",
        annotations(
            title = "Inspect retention uncertainty",
            read_only_hint = true
        )
    )]
    async fn retention(
        &self,
        Parameters(params): Parameters<RetentionParams>,
    ) -> Result<String, String> {
        crate::retention::inspect(&params.path).await
    }

    /// Rank explicit candidate paths by private allocation or standalone
    /// conservative reclaim lower bound. Read-only, no marking. Unknown
    /// values are null and sort last; known zero stays zero. The complete
    /// union is estimated once: never add candidate bounds. Ranking does
    /// not establish ownership, disposability or immediate recovered space.
    #[tool(
        name = "pwrdisk_rank",
        annotations(title = "Rank cleanup candidates", read_only_hint = true)
    )]
    async fn rank(
        &self,
        Parameters(params): Parameters<RankParams>,
    ) -> Result<String, String> {
        self.with(|session| session.rank(&params))
    }

    /// Scan a directory tree and keep the result for the other tools. With
    /// no arguments it scans the home directory, which takes seconds; disk:
    /// true scans the whole volume. Returns totals, the volume's free space
    /// and how much of its used space this scan explains, the top level
    /// largest first, and the biggest things that could plausibly go.
    /// Scanning the same root again replaces the earlier scan. When a
    /// PwrDisk window is open it does the scanning and shows the result;
    /// the window's tree is then what every other tool reads.
    #[tool(
        name = "pwrdisk_scan",
        annotations(title = "Scan a directory", read_only_hint = true)
    )]
    async fn scan(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<String, String> {
        match self.with(|s| s.prepare_scan(&params))? {
            ScanPlan::Local { root, options } => {
                let scan = tokio::task::spawn_blocking(move || {
                    run_scan(root, options)
                })
                .await
                .map_err(|error| format!("the scan task failed: {error}"))??;
                Ok(self.with(|s| s.store_scan(scan)))
            }
            ScanPlan::Window { root } => {
                // The window walks; the session is unlocked between looks
                // so pwrdisk_status still answers meanwhile.
                let started = Instant::now();
                loop {
                    if let Some(report) = self.with(|s| s.window_scan(&root))? {
                        return Ok(report);
                    }
                    if started.elapsed() > SCAN_WAIT {
                        return Err(format!(
                            "The window is still scanning {} after {} s. \
                             pwrdisk_status says when it is done; then \
                             pwrdisk_ls reads the tree.",
                            root.display(),
                            started.elapsed().as_secs()
                        ));
                    }
                    tokio::time::sleep(SCAN_POLL).await;
                }
            }
        }
    }

    /// List a scanned directory, largest first: each entry's share, size,
    /// kind of data, whether its space can be had back and why, and when it
    /// was last written. Sorts by listed allocation, not predicted recovery;
    /// compare explicit candidates with `pwrdisk_rank`. Never moves the view.
    #[tool(
        name = "pwrdisk_ls",
        annotations(title = "List a directory", read_only_hint = true)
    )]
    async fn ls(
        &self,
        Parameters(params): Parameters<LsParams>,
    ) -> Result<String, String> {
        self.with(|s| s.ls(&params))
    }

    /// The largest entries matching the filters, anywhere beneath a path or
    /// every scanned root: files over a size, everything reclaimable
    /// (caches, build output, package stores), one kind of data, a name,
    /// or what nobody has written to in a while. A matching directory is
    /// listed once, without its contents. Sorts by listed allocation;
    /// compare explicit candidates with `pwrdisk_rank` before recommending.
    #[tool(
        name = "pwrdisk_find",
        annotations(title = "Find the largest matches", read_only_hint = true)
    )]
    async fn find(
        &self,
        Parameters(params): Parameters<FindParams>,
    ) -> Result<String, String> {
        self.with(|s| s.find(&params))
    }

    /// Find typed build output, caches, dependencies and temporary/scratch
    /// locations with evidence, family/subtype filters, size/age filters and
    /// oldest/largest ordering. jvm includes Scala, Gradle, Maven and Coursier;
    /// cargo identifies Cargo output through metadata checks. Other detectors
    /// expose name/layout evidence without claiming metadata verification.
    /// Nested types remain visible; union totals avoid double counting.
    /// Never navigates, marks or authorizes removal. Use estimate/retention
    /// before promising recovery. Age is file mtime, not proof of inactivity.
    #[tool(
        name = "pwrdisk_find_outputs",
        annotations(
            title = "Find typed output candidates",
            read_only_hint = true
        )
    )]
    async fn find_outputs(
        &self,
        Parameters(params): Parameters<outputs::Params>,
    ) -> Result<String, String> {
        let server = self.clone();
        tokio::task::spawn_blocking(move || {
            server.with(|s| s.find_outputs(&params))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Estimate removing a supplied path set without changing marks, scan
    /// root, navigation or removal intents. Returns accepted/covered/refused
    /// paths, allocation, selected-set physical bounds and scan limitations.
    /// Bounded directory peers identify retained clones within the scan and
    /// quantify union savings; these are inspection leads, not advice to remove.
    #[tool(
        name = "pwrdisk_estimate",
        annotations(
            title = "Estimate hypothetical path set",
            read_only_hint = true
        )
    )]
    async fn estimate(
        &self,
        Parameters(params): Parameters<EstimateParams>,
    ) -> Result<String, String> {
        self.with(|s| s.estimate_paths(&params))
    }

    /// Show a path in the PwrDisk window, selected in its directory, so
    /// the user sees what you are looking at. Opens a window when none is.
    #[tool(
        name = "pwrdisk_show",
        annotations(
            title = "Show in the window",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn show(
        &self,
        Parameters(params): Parameters<ShowParams>,
    ) -> Result<String, String> {
        self.with(|s| s.show_path(&params))
    }

    /// Mark paths for removal, once the user has agreed the list or a
    /// standing instruction of theirs covers it. Nothing is deleted: marks
    /// appear in the
    /// PwrDisk window when one is open, and the user removes them from the
    /// review screen. Returns the plan: what is marked, what the guards
    /// keep back and why (the home directory, an account's folder, a mount
    /// point, a system tree), what is covered by another mark, and
    /// scan-derived reclaim bounds before retention. These are not
    /// guaranteed recovered space. A linked window refuses a mark that
    /// would take the list past the capacity limit; only the person can
    /// mark past it, in the window.
    #[tool(
        name = "pwrdisk_mark",
        annotations(
            title = "Mark for removal",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn mark(
        &self,
        Parameters(params): Parameters<PathsParams>,
    ) -> Result<String, String> {
        self.with(|s| s.mark(&params))
    }

    /// Preview exactly the marked paths in an idle local window for a
    /// required explicit mode. Does not remove, navigate or change marks.
    /// Show the preview and consequences to the user and await confirmation.
    #[tool(
        name = "pwrdisk_removal_preview",
        annotations(
            title = "Preview a removal",
            read_only_hint = true,
            destructive_hint = false
        )
    )]
    async fn removal_preview(
        &self,
        Parameters(params): Parameters<RemovalPreviewParams>,
    ) -> Result<String, String> {
        self.with(|session| session.removal_preview(&params))
    }

    /// DESTRUCTIVE WRITE: ask the human, through the MCP client, to approve
    /// the preview's exact paths, mode, bytes and consequences. Show the
    /// proposal first; the tool arguments do not authorize removal. Approval to
    /// mark is insufficient. Trash may be recoverable until emptied;
    /// permanent deletion cannot be undone. Never use computer use to approve.
    /// The app validates the single-use preview and configured capacity limit.
    /// Returns when started; poll status for completion. Never retry automatically.
    #[tool(
        name = "pwrdisk_remove",
        annotations(
            title = "Remove confirmed paths",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn remove(
        &self,
        Parameters(params): Parameters<RemoveParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        if let Some(outcome) = self.with(|s| s.removal_outcome(&params)) {
            return Ok(outcome);
        }
        let (message, deadline) =
            self.with(|session| session.removal_confirmation_message(&params))?;
        if !context
            .peer
            .supported_elicitation_modes()
            .contains(&rmcp::service::ElicitationMode::Form)
        {
            return Err("This MCP client does not support interactive form confirmation. No MCP removal request was sent; check pwrdisk_status for any native UI operation. Use human review in PwrDisk; never approve it through computer use.".into());
        }
        // An empty form asks for a decision, not data entry. Messaging hosts
        // can offer Allow / Decline without inventing a required field value.
        // Never label this as a host tool grant: Auto must still ask a person.
        let requested_schema = ElicitationSchema::builder()
            .build()
            .map_err(ToString::to_string)?;
        let request = ElicitRequestParams::FormElicitationParams {
            meta: None,
            message,
            requested_schema,
        };
        let answer = async {
            let mut handle = context
                .peer
                .send_cancellable_request(
                    ServerRequest::ElicitRequest(ElicitRequest::new(request)),
                    PeerRequestOptions::default(),
                )
                .await
                .map_err(|error| error.to_string())?;
            let response = tokio::select! {
                biased;
                () = context.ct.cancelled() => None,
                () = tokio::time::sleep_until(deadline.into()) => None,
                response = &mut handle.rx => Some(response),
            };
            match response {
                Some(Ok(Ok(ClientResult::ElicitResult(answer)))) => Ok(answer),
                Some(response) => {
                    Err(format!("Invalid confirmation response: {response:?}"))
                }
                None => {
                    // Withdraw the actual host card, rather than just dropping
                    // a future and leaving an apparently live Allow button.
                    let _ = handle
                        .cancel(Some(
                            "Removal approval expired or was cancelled".into(),
                        ))
                        .await;
                    Err("Removal approval expired or was cancelled".into())
                }
            }
        }
        .await;
        // The person may have approved in the native UI while this form
        // was open. Cancellation describes the form, not that operation.
        if let Some(outcome) = self.with(|s| s.removal_outcome(&params)) {
            return Ok(outcome);
        }
        let answer = answer.map_err(|error| format!("MCP confirmation failed ({error}). No MCP removal request was sent; native UI outcome could not be verified. Check pwrdisk_status. Never retry automatically."))?;
        if !confirmed_by_user(&answer) {
            return Err("MCP confirmation was declined or cancelled. No MCP removal request was sent; native UI outcome could not be verified. Check pwrdisk_status before considering a new preview. Never retry automatically.".into());
        }
        // Cancellation and expiration may race a positive answer. Neither
        // an old card nor a late callback authorizes starting a new write.
        if context.ct.is_cancelled() || Instant::now() >= deadline {
            return Err("The removal approval expired or was cancelled. No MCP removal request was sent. Check pwrdisk_status; never retry automatically.".into());
        }
        self.with(|session| session.remove(&params))
    }

    /// Unmark paths, or everything when no paths are given. Returns the
    /// plan that remains.
    #[tool(
        name = "pwrdisk_unmark",
        annotations(
            title = "Unmark",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn unmark(
        &self,
        Parameters(params): Parameters<PathsParams>,
    ) -> Result<String, String> {
        self.with(|s| s.unmark(&params))
    }

    /// Open PwrDisk's review screen over the marks, starting a window if
    /// none is open; tell the user first, since it comes forward on their
    /// screen. The user decides there: Enter moves the marks to the trash,
    /// whose space returns once it is emptied; permanent deletion always
    /// asks first. Call `pwrdisk_status` afterwards for what was freed.
    #[tool(
        name = "pwrdisk_review",
        annotations(
            title = "Open the review screen",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn review(&self) -> Result<String, String> {
        self.with(Session::review)
    }
}

#[prompt_router]
impl Server {
    /// How to free space with PwrDisk: measure, scan, walk, propose, mark
    /// what the user agrees to, and hand the decision back on the review
    /// screen. Carries this system's cautions.
    #[prompt(name = "free-space")]
    async fn free_space(&self) -> GetPromptResult {
        skill_prompt("pwrdisk-free-space")
    }

    /// Where the space went, read-only: scan, walk, account for what the
    /// files do not explain, and explain. Carries this system's cautions.
    #[prompt(name = "explain-usage")]
    async fn explain_usage(&self) -> GetPromptResult {
        skill_prompt("pwrdisk-explain-usage")
    }
}

/// Only an explicit positive answer from the client counts as approval.
fn confirmed_by_user(answer: &rmcp::model::ElicitResult) -> bool {
    answer.action == ElicitationAction::Accept
        && answer.content.as_ref().is_some_and(|value| {
            value.as_object().is_some_and(serde_json::Map::is_empty)
        })
}

/// A skill as one prompt message, with this system's reference after it.
fn skill_prompt(name: &str) -> GetPromptResult {
    let skill = skills::named(name).expect("a skill listed in SKILLS");
    GetPromptResult::new(vec![PromptMessage::new_text(
        Role::User,
        skills::prompt_text(skill),
    )])
    .with_description(skill.description())
}

/// A JSON-RPC error for a request the skills extension cannot answer.
fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, message.into(), None)
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .enable_resources()
            .build();
        // Clients that know the skills extension list the skills with
        // `skills/list`; the rest find the same files as resources.
        capabilities.extensions = Some(
            std::iter::once((skills::EXTENSION.to_string(), JsonObject::new()))
                .collect(),
        );
        ServerConfig::new(capabilities)
            .with_server_info(
                Implementation::new("pwrdisk", env!("CARGO_PKG_VERSION"))
                    .with_title("PwrDisk")
                    .with_website_url("https://github.com/pwrdrvr/PwrDisk"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    /// Learn the client's name before answering: a client may call a tool
    /// as soon as it has the answer, before `initialized` arrives, and the
    /// window's header names the agent from the first `hello`.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.with(|s| s.set_client(&request.client_info.name));
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(resources()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let Some(text) = skills::read(&request.uri) else {
            return Err(ErrorData::resource_not_found(
                format!("no resource at {}", request.uri),
                None,
            ));
        };
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri)
                .with_mime_type("text/markdown"),
        ])
        .into())
    }

    /// `skills/list` and `skills/get`, the skills extension's requests.
    /// Anything else is not a method this server has.
    async fn on_custom_request(
        &self,
        request: CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> Result<CustomResult, ErrorData> {
        custom(&request.method, request.params.as_ref())
    }
}

/// Every skill file as a resource; a `SKILL.md` carries the skill's name
/// and description.
pub fn resources() -> Vec<Resource> {
    SKILLS
        .iter()
        .flat_map(|skill| {
            skill.files.iter().map(move |file| {
                let address = skills::uri(skill.name, file.path);
                let size = u64::try_from(file.text.len()).unwrap_or(u64::MAX);
                let resource = if file.path == "SKILL.md" {
                    Resource::new(address, skill.name)
                        .with_description(skill.description())
                } else {
                    Resource::new(address, file.path)
                };
                resource.with_mime_type("text/markdown").with_size(size)
            })
        })
        .collect()
}

/// Answer a custom request: the skills extension, or not found.
pub fn custom(
    method: &str,
    params: Option<&serde_json::Value>,
) -> Result<CustomResult, ErrorData> {
    if params
        .and_then(|params| params.get("cursor"))
        .is_some_and(|cursor| !cursor.is_null())
    {
        return Err(invalid("this server lists everything in one page"));
    }
    match method {
        "skills/list" => {
            let skills: Vec<serde_json::Value> =
                SKILLS.iter().map(skills::Skill::manifest).collect();
            Ok(CustomResult::new(serde_json::json!({ "skills": skills })))
        }
        "skills/get" => {
            let address = params
                .and_then(|params| params.get("uri"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid("skills/get needs a uri"))?;
            let skill = skills::by_uri(address)
                .ok_or_else(|| invalid(format!("no skill at {address}")))?;
            Ok(CustomResult::new(
                serde_json::json!({ "skill": skill.manifest() }),
            ))
        }
        _ => Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            method.to_string(),
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every supported find kind must agree across the advertised schema,
    /// JSON deserialization and the core classifier's category names.
    #[test]
    fn find_category_schema_and_requests_agree_with_core_kinds() {
        use disktree_core::classify::Category;

        let tools = Server::tool_router().list_all();
        let find = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_find")
            .expect("find");
        let categories = find.input_schema["$defs"]["CategoryName"]["enum"]
            .as_array()
            .expect("category enum");
        assert_eq!(categories.len(), Category::LEGEND.len() + 1);
        for category in Category::LEGEND.into_iter().chain([Category::Other]) {
            let name = category.name().replace(' ', "-");
            assert!(
                categories.contains(&serde_json::json!(name)),
                "find schema must advertise {name}"
            );
            let params: FindParams =
                serde_json::from_value(serde_json::json!({"category": name}))
                    .expect("advertised category must deserialize");
            assert_eq!(params.category.map(Category::from), Some(category));
        }
    }

    /// Every skill `skills/list` names is the one `skills/get` returns, and
    /// every file it lists is a resource that reads back with that size
    /// and digest.
    #[test]
    fn skills_list_get_and_resources_agree() {
        let listed = custom("skills/list", None).expect("list").0;
        let entries = listed["skills"].as_array().expect("skills");
        assert_eq!(entries.len(), SKILLS.len());
        let resources = resources();
        let mut covered = 0;
        for entry in entries {
            let address = entry["uri"].as_str().expect("uri");
            let got = custom(
                "skills/get",
                Some(&serde_json::json!({ "uri": address })),
            )
            .expect("get")
            .0;
            assert_eq!(&got["skill"], entry);
            for file in entry["resources"].as_array().expect("files") {
                let address = file["uri"].as_str().expect("uri");
                let text = skills::read(address).expect("readable");
                assert_eq!(file["size"], text.len());
                assert_eq!(file["digest"], skills::digest(text));
                let resource = resources
                    .iter()
                    .find(|resource| resource.uri == address)
                    .expect("listed as a resource");
                assert_eq!(resource.size, Some(text.len() as u64));
                covered += 1;
            }
        }
        assert_eq!(covered, resources.len(), "no resource outside a skill");
    }

    #[test]
    fn output_inventory_advertises_generic_types_and_age_ordering() {
        let tools = Server::tool_router().list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_find_outputs")
            .expect("one generic query must cover every output type");
        assert_eq!(
            tool.annotations.as_ref().expect("read tool").read_only_hint,
            Some(true)
        );
        let schema = serde_json::to_value(&tool.input_schema).expect("schema");
        for key in [
            "older_than_days",
            "min_bytes",
            "order",
            "offset",
            "types",
            "minimum_evidence",
        ] {
            assert!(schema["properties"].get(key).is_some(), "{key}");
        }
        let orders = schema["$defs"]["Order"]["enum"]
            .as_array()
            .expect("order enum");
        assert_eq!(orders.len(), 2);
        for order in ["oldest", "largest"] {
            assert!(orders.contains(&serde_json::json!(order)));
            let params: outputs::Params = serde_json::from_value(
                serde_json::json!({"order": order, "older_than_days": 30}),
            )
            .expect("advertised order deserializes");
            assert_eq!(
                serde_json::to_value(params.order).expect("order"),
                order
            );
        }
        for invalid in [
            serde_json::json!({"order": "newest"}),
            serde_json::json!({"older_than_days": -1}),
            serde_json::json!({"offset": -1}),
            serde_json::json!({"unrecognized_policy": true}),
        ] {
            assert!(
                serde_json::from_value::<outputs::Params>(invalid).is_err()
            );
        }
    }

    #[test]
    fn bad_skill_requests_are_refused() {
        for params in [
            None,
            Some(serde_json::json!({})),
            Some(serde_json::json!({ "uri": 7 })),
            Some(serde_json::json!({
                "uri": "skill://pwrdisk-free-space/references/macos.md"
            })),
            Some(serde_json::json!({ "uri": "file:///etc/passwd" })),
        ] {
            let error =
                custom("skills/get", params.as_ref()).expect_err("not a skill");
            assert_eq!(error.code, ErrorCode::INVALID_PARAMS, "{params:?}");
        }
        let paged = custom(
            "skills/list",
            Some(&serde_json::json!({ "cursor": "next" })),
        )
        .expect_err("one page");
        assert_eq!(paged.code, ErrorCode::INVALID_PARAMS);
        let unknown = custom("skills/delete", None).expect_err("no method");
        assert_eq!(unknown.code, ErrorCode::METHOD_NOT_FOUND);
    }

    #[test]
    fn the_server_declares_the_skills_extension() {
        let server = Server::new(Session::detached(
            std::env::temp_dir().join("pwrdisk-no-window.sock"),
            None,
        ));
        let info = server.get_info();
        let extensions =
            info.capabilities.extensions.expect("extensions declared");
        assert!(extensions.contains_key(skills::EXTENSION));
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.prompts.is_some());
        let instructions = info.instructions.expect("instructions");
        for skill in SKILLS {
            assert!(instructions.contains(&skill.uri()), "{}", skill.name);
        }
    }

    #[tokio::test]
    async fn skills_are_readable_directly_from_tool_discovery() {
        let server = Server::new(Session::detached(
            std::env::temp_dir().join("pwrdisk-no-window.sock"),
            None,
        ));
        for skill in SKILLS {
            for file in skill.files {
                let text = server
                    .read_skill(Parameters(SkillParams {
                        uri: skills::uri(skill.name, file.path),
                    }))
                    .await
                    .expect("tool reads exact resource");
                assert_eq!(text, file.text);
            }
        }
        assert!(
            server
                .read_skill(Parameters(SkillParams {
                    uri: "file:///etc/passwd".into()
                }))
                .await
                .is_err()
        );
        let tools = Server::tool_router().list_all();
        for name in ["pwrdisk_read_skill", "pwrdisk_retention", "pwrdisk_rank"]
        {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("discoverable");
            assert_eq!(
                tool.annotations
                    .as_ref()
                    .expect("annotations")
                    .read_only_hint,
                Some(true)
            );
        }
        let status = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_status")
            .expect("status");
        assert!(
            status
                .description
                .as_ref()
                .expect("onboarding")
                .contains("serverNameInAgent")
        );
    }
    #[test]
    fn removal_is_a_destructive_non_idempotent_write_and_listing_is_read_only()
    {
        let tools = Server::tool_router().list_all();
        let remove = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_remove")
            .expect("remove");
        let flags = remove.annotations.as_ref().expect("write annotations");
        assert_eq!(flags.read_only_hint, Some(false));
        assert_eq!(flags.destructive_hint, Some(true));
        assert_eq!(flags.idempotent_hint, Some(false));
        let estimate = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_estimate")
            .expect("estimate");
        assert_eq!(
            estimate.annotations.as_ref().expect("read").read_only_hint,
            Some(true)
        );
        let listing = tools
            .iter()
            .find(|tool| tool.name == "pwrdisk_ls")
            .expect("ls");
        assert_eq!(
            listing.annotations.as_ref().expect("read").read_only_hint,
            Some(true)
        );
        assert!(
            !listing.input_schema["properties"]
                .as_object()
                .expect("properties")
                .contains_key("show")
        );
    }

    #[test]
    fn only_explicit_client_acceptance_of_the_empty_form_confirms_removal() {
        use rmcp::model::ElicitResult;
        for action in [
            ElicitationAction::Accept,
            ElicitationAction::Decline,
            ElicitationAction::Cancel,
        ] {
            for content in [
                None,
                Some(serde_json::json!({})),
                Some(serde_json::json!({"confirmed":false})),
                Some(serde_json::json!({"confirmed":true})),
            ] {
                let mut answer = ElicitResult::new(action.clone());
                answer.content.clone_from(&content);
                let expected = action == ElicitationAction::Accept
                    && content == Some(serde_json::json!({}));
                assert_eq!(confirmed_by_user(&answer), expected);
            }
        }
    }
}
