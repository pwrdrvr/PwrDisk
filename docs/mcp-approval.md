# Human approval for MCP removals

PwrDisk uses standard MCP form elicitation (`elicitation/create`) to ask the
person about a concrete operation. The form has an empty object schema:
there are no values to enter. The host presents Accept/Allow and Decline.
In PwrAgent this also works through its messaging approval controls.

An idle linked **local** PwrDisk window is still required. Native Review
remains available. Read-only MCP access needs neither approval nor a window.
There is no headless or SSH removal and no computer-use approval route.

## Agent sequence

1. Agree the candidate list and mark it. Marking does not authorize removal.
2. Call `pwrdisk_removal_preview` with the explicit mode: `trash`,
   `permanent` or `owner_cleanup`. For owner cleanup first show the proposals
   and get the person's per-path choices, then preview those decisions.
3. Show the proposal. Call `pwrdisk_remove` with its exact `preview_id`,
   ordered `paths`, `mode` and `confirmation` text to **request** approval.
   That text encodes the operation; it is not evidence of consent.
4. The host shows the server's confirmation. Only the person may answer.
   Never submit elicitation responses on their behalf. Neither an earlier
   chat message, a standing cleanup instruction, tool Allow-all, Auto nor
   Full Access replaces this decision.
5. Read `pwrdisk_status` for completion, failures and measured volume change.
   An accepted proposal or a started operation does not prove bytes freed.

The card includes the stable preview ID, scan root, expiry, full selected
paths, mode and consequences, listed allocation, estimate uncertainty and
capacity limits. Owner cleanup also includes its exact commands and actual
scope, including unselected siblings, and separate file-deletion decisions.
Trash has no permanent-delete fallback. Owner failure has no file-deletion
fallback. Listed allocation and reclaim estimates are not immediate savings:
sharing, snapshots, open files and concurrent activity can retain space.

## Host contract and trust boundary

The client must negotiate form elicitation, route the request to the human
who owns the originating session/thread and return that person's response
on the same JSON-RPC request. PwrDisk accepts only
`{"action":"accept","content":{}}`; decline, cancel, malformed responses,
unsupported clients and transport errors send no MCP removal request.
The server never marks its form as a host tool grant or asks for a
remembered permission. A host must not automatically accept this form.

PwrAgent's generic upstream relay associates the form with the exact
upstream client and the sole in-flight originating request. Ambiguous calls,
ended turns, revoked selections, Job Monitors and headless automations fail
closed. Messaging uses the bound conversation, allowed actor and existing
approval RBAC. Text approval requires the displayed `approve <card code>`
phrase; a bare "yes" or another card's phrase does not approve it.
A callback must identify its original pending card; an old
button must not answer a newer request. This requires PwrAgent's
exact-card callback routing fix in addition to its existing upstream relay.
Clients that cannot present the confirmation must use native human Review.

MCP does not provide a cryptographic attestation of a human click. PwrDisk
trusts the connected host to enforce that boundary. Its same-user local IPC
is not a sandbox against malicious programs already running as that OS user.
No tool argument, confirmation boolean or annotation establishes authority.

## Expiry, cancellation and retries

A preview lasts at most five minutes from preparation in the window. The
server consumes its session copy before asking and bounds the wait by the
remaining lifetime. Expiry or caller cancellation withdraws the host form.
The window independently consumes and validates its copy before execution.
Changing the selection, mode, scan, decisions or safety policy requires a
fresh preview and a new human decision. Replacing a selected object or
changing its resolved parent or root invalidates its identity binding. The
worker checks that binding before batch preflight and each target dispatch.
Existing no-follow traversal and mount/removal guards still apply.

These are identity checks, not a filesystem snapshot or an atomic lock on
other processes. Directory approval includes contents present at execution;
concurrent changes inside an approved directory are not frozen. A filesystem
that cannot supply identity fails closed for this MCP approval route.

Pending approvals are process/session-local. A host disconnect or restart
does not persist or replay acceptance. A still-live host may restore a card,
but the originating request and unexpired window preview must both remain
valid. After reconnect, inspect status rather than retrying removal.
An exact operation already running or completed returns its recorded outcome
without another write, including when the person acted through native Review
while the form was open. Unknown outcome means unknown; a cancelled form
does not prove that no native operation ran. Ask again only for a fresh
proposal after reconciling the outcome with the person.

## Verification

`cargo xtask test` covers the stdio request/elicitation/response exchange,
unsupported clients, malformed/negative answers, caller cancellation,
consumption, expiry and replay; the headless app harness exercises real IPC,
path replacement, changed plans/policy and actual disposable-file removal.
Filesystem properties cover identity replacement and worker preflight.

PwrAgent has an opt-in cross-project smoke test:

```sh
# In this PwrDisk checkout; builds but never opens a window.
cargo build -p disktree-app
# In the isolated PwrAgent checkout, use this checkout's absolute binary path.
PWRDISK_APPROVAL_TEST_BINARY=/absolute/PwrDisk/target/debug/pwrdisk \
  pnpm test apps/desktop/src/main/__tests__/messaging-controller.test.ts \
  -t 'round trips PwrDisk deletion through fake messaging'
```

That test runs the actual PwrDisk MCP binary and SDK with a fixture-only
`PWRDISK_LINK`, the real messaging controller and simulated human callbacks.
Its stand-in native window deletes only owned temporary files. It verifies
acceptance, denial, retries and an untouched unmarked neighbour. It does not
contact Telegram, start a native window or test installed application builds.
