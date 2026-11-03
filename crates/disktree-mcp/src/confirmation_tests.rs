//! Exercise the server/client confirmation exchange without removing data.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use disktree_link::{
    Incoming, Listener, Mark, RemovalChoice, RemovalPreview, Request, Response,
    State, VERSION,
};
use rmcp::ServiceExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::server::Server;
use crate::session::Session;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_interactive_user_confirmation_reaches_the_removal_request() {
    for mode_name in ["permanent", "owner_cleanup"] {
        for (supports_form, action, confirmed, expected) in [
            (false, "accept", true, 0),
            (true, "decline", true, 0),
            (true, "cancel", true, 0),
            (true, "accept", false, 0),
            (true, "accept", true, 1),
        ] {
            let temp = tempfile::tempdir().expect("fixture");
            let file = temp.path().join("candidate");
            std::fs::write(&file, b"fixture").expect("fixture data");
            #[cfg(not(windows))]
            let socket = temp.path().join("window.sock");
            #[cfg(windows)]
            let socket = std::path::PathBuf::from(format!(
                r"\\.\pipe\pwrdisk-confirmation-{}",
                temp.path().file_name().expect("name").to_string_lossy()
            ));
            let listener =
                Listener::bind_at(&socket).expect("fixture listener");
            let calls = Arc::new(AtomicUsize::new(0));
            let stopped = Arc::new(AtomicBool::new(false));
            let (count, stop) = (Arc::clone(&calls), Arc::clone(&stopped));
            let root = temp.path().to_path_buf();
            let candidate = file.clone();
            let window = std::thread::spawn(move || {
                let state = State {
                    version: VERSION,
                    app_version: "0.13.0".into(),
                    root: root.clone(),
                    current: root.clone(),
                    screen: "explore".into(),
                    scanning: false,
                    scanned: true,
                    estimate: None,
                    marks: vec![Mark {
                        path: candidate.clone(),
                        bytes: 7,
                        is_dir: false,
                    }],
                    space: None,
                    removal: None,
                    removals: None,
                    tree: None,
                    insights: vec![],
                    remote: None,
                };
                while !stop.load(Ordering::Relaxed) {
                    for incoming in listener.drain() {
                        if let Incoming::Request { request, reply, .. } =
                            incoming
                        {
                            let answer = match request {
                                Request::Hello { .. } | Request::State => {
                                    Response::State(state.clone())
                                }
                                Request::PrepareRemoval { mode } => {
                                    Response::RemovalPreview(RemovalPreview {
                                        preview_id: "fixture-preview".into(),
                                        root: root.clone(),
                                        mode,
                                        paths: vec![candidate.clone()],
                                        bytes: 7,
                                        estimate:
                                            disktree_link::SelectionEstimate {
                                                paths: vec![candidate.clone()],
                                                lower: 0,
                                                upper: None,
                                                on_volume: 0,
                                                partial: true,
                                                shared: 0,
                                            },
                                        limit_percent: 5,
                                        volumes: vec![],
                                        expires_in_seconds: 300,
                                        cleanup: if mode
                                            == RemovalChoice::OwnerCleanup
                                        {
                                            vec![disktree_core::cleanup::Operation {
                                        owner: disktree_core::cleanup::Owner::Uv,
                                        version: "uv 0.11.0".into(),
                                        selected: vec![candidate.clone()],
                                        cache_root: root.clone(), scope: candidate.clone(),
                                        command: disktree_core::cleanup::Command {
                                            executable: "/fixture/uv".into(), arguments: vec!["cache".into(), "clean".into()],
                                            environment: std::collections::BTreeMap::new(), working_directory: root.clone(),
                                        },
                                        effects: "Entire cache scope including unselected siblings".into(),
                                        limitations: "No raw deletion fallback; APFS snapshots can retain blocks".into(),
                                    }]
                                        } else {
                                            vec![]
                                        },
                                    })
                                }
                                Request::Remove(_) => {
                                    count.fetch_add(1, Ordering::Relaxed);
                                    Response::State(state.clone())
                                }
                                _ => Response::error(
                                    "unexpected fixture request",
                                ),
                            };
                            reply.send(answer);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            let server = Server::new(Session::detached(socket, None));
            let (client_io, server_io) = tokio::io::duplex(65_536);
            let task = tokio::spawn(async move {
                let service = server.serve(server_io).await.expect("server");
                let _ = service.waiting().await;
            });
            let (input, mut output) = tokio::io::split(client_io);
            let mut input = BufReader::new(input);
            let capabilities = if supports_form {
                json!({"elicitation":{"form":{}}})
            } else {
                json!({})
            };
            send(
                &mut output,
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                    "protocolVersion":"2025-06-18","capabilities":capabilities,
                    "clientInfo":{"name":"fixture-client","version":"1"}
                }}),
            )
            .await;
            assert_eq!(receive(&mut input).await["id"], 1);
            send(
                &mut output,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            )
            .await;
            send(&mut output, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"pwrdisk_removal_preview","arguments":{"mode":mode_name}
        }})).await;
            let preview = receive(&mut input).await;
            assert_ne!(preview["result"]["isError"], true, "{preview}");
            send(&mut output, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"pwrdisk_remove","arguments":{
                "preview_id":"fixture-preview","mode":mode_name,"paths":[file],
                "confirmation": if mode_name == "owner_cleanup" { RemovalChoice::OwnerCleanup.confirmation() } else { RemovalChoice::Permanent.confirmation() }
            }
        }})).await;
            let mut asked = false;
            loop {
                let response = receive(&mut input).await;
                if response["method"] == "elicitation/create" {
                    asked = true;
                    let message = response["params"]["message"]
                        .as_str()
                        .expect("confirmation text");
                    assert!(message.contains(&file.display().to_string()));
                    if mode_name == "owner_cleanup" {
                        for expected in [
                            "/fixture/uv",
                            "cache",
                            "clean",
                            "unselected siblings",
                            "No raw deletion fallback",
                            "APFS",
                        ] {
                            assert!(message.contains(expected), "{message}");
                        }
                    } else {
                        assert!(message.contains("permanently deletes"));
                        assert!(message.contains("cannot be undone"));
                    }
                    assert!(message.contains("5% of each volume"));
                    // This simulates the client's user response, never model input.
                    send(
                        &mut output,
                        json!({"jsonrpc":"2.0","id":response["id"],"result":{
                            "action":action,"content":{"confirmed":confirmed}
                        }}),
                    )
                    .await;
                } else if response["id"] == 3 {
                    assert_eq!(
                        response["result"]["isError"] == true,
                        expected == 0,
                        "{response}"
                    );
                    break;
                }
            }
            assert_eq!(asked, supports_form);
            assert_eq!(calls.load(Ordering::Relaxed), expected);
            assert_eq!(
                std::fs::read(&file).expect("fixture is untouched"),
                b"fixture"
            );
            task.abort();
            let _ = task.await;
            stopped.store(true, Ordering::Relaxed);
            window.join().expect("fixture window");
        }
    }
}

async fn send(output: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    let line = format!("{value}\n");
    output.write_all(line.as_bytes()).await.expect("send");
    output.flush().await.expect("flush");
}

async fn receive(input: &mut (impl tokio::io::AsyncBufRead + Unpin)) -> Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), input.read_line(&mut line))
        .await
        .expect("protocol response in time")
        .expect("read");
    serde_json::from_str(&line).expect("JSON-RPC")
}
