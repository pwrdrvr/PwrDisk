//! The MCP surface: eight tools, and the skills that say how to use them,
//! as prompts, resources and the skills extension.
//!
//! Each tool is a thin call into [`Session`]; the words an agent reads come
//! from there. Tools that only look are marked read-only, so a client can
//! run them without asking. Nothing here removes a file: marking is not
//! destructive, and the review screen belongs to the person.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CustomRequest, CustomResult, ErrorCode, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, JsonObject, ListResourcesResult,
    PromptMessage, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, Role, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{
    ErrorData, ServerHandler, prompt, prompt_handler, prompt_router, tool,
    tool_handler, tool_router,
};

use crate::session::{
    FindParams, LsParams, PathsParams, ScanParams, ScanPlan, Session,
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
PwrDisk measures a directory tree the way the disk does: allocated \
blocks, hardlinks once, hidden entries included. Start with \
pwrdisk_status, then pwrdisk_scan (no arguments: the home directory; \
disk: true: the whole volume, which can take minutes). Walk with \
pwrdisk_ls, largest first; pwrdisk_find lists the largest files or every \
cache, build directory and package store at once. \
Looking needs no permission; acting does. Agree the list with the user \
before pwrdisk_mark, unless a standing instruction of theirs covers it, \
and say so before pwrdisk_review brings the window forward. pwrdisk_mark \
returns the plan: what the guards keep back and why, and the free space \
it would give back. You never delete: not with these tools, not with a \
shell command, and not with a tool's own clean command the user has not \
approved. pwrdisk_review opens PwrDisk's review screen, where the user \
presses Enter; marks go to the Trash or Recycle Bin, whose space returns \
once it is emptied. pwrdisk_status afterwards says what was freed. \
When a PwrDisk window is open its tree is what you read, and a path \
outside its root needs a pwrdisk_scan from higher up, which the window \
follows; pass show: true to pwrdisk_ls or call pwrdisk_show to move the \
view. The tools work on this computer only: a window showing an SSH host \
is not followed. On Windows there is no window link yet, so give the user \
the list to mark in PwrDisk themselves. \
Skills, as resources, through skills/list, and as the prompts \
explain-usage and free-space: skill://pwrdisk-explain-usage/SKILL.md \
answers where the space went without changing anything; \
skill://pwrdisk-free-space/SKILL.md frees space and holds the \
confirmation rules; its references/macos.md, references/linux.md and \
references/windows.md say what each system hides from a scan and what \
never to mark. Read the one for this system before marking.";

/// The server: one session behind a lock, so the tools can run from any
/// protocol task while scans run off to the side.
#[derive(Clone)]
pub struct Server {
    session: Arc<Mutex<Session>>,
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
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    /// Run `f` with the session. A poisoned lock means a tool panicked; the
    /// state is still worth more than nothing.
    pub fn with<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let mut session =
            self.session.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut session)
    }
}

#[tool_router]
impl Server {
    /// What this session knows: scans held, the window if one is open, what
    /// is marked, the last removal, and every volume's free space. Start
    /// here.
    #[tool(
        name = "pwrdisk_status",
        annotations(title = "PwrDisk status", read_only_hint = true)
    )]
    async fn status(&self) -> String {
        self.with(Session::status)
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
    /// was last written. show: true moves the PwrDisk window there.
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
    /// listed once, without its contents.
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

    /// Show a path in the PwrDisk window, selected in its directory, so
    /// the user sees what you are looking at. Opens a window when none is.
    #[tool(
        name = "pwrdisk_show",
        annotations(title = "Show in the window", read_only_hint = true)
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
    /// keep back and why (the home directory, a mount point, a system
    /// tree), what is covered by another mark, and the free space it gives
    /// back on the volume.
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
}
