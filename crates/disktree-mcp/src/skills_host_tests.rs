//! A host that follows the skills extension finds and loads every skill.
//!
//! These drive the server over the wire the way the extension's host
//! sequence does: see the declaration, list, look a skill up by URI, read
//! each file through `resources/read`, and verify its size, digest and
//! frontmatter against the manifest. Both connection styles are covered:
//! `server/discover` with per-request metadata, and `initialize`.

use serde_json::{Value, json};
use tokio::io::{BufReader, DuplexStream, ReadHalf, WriteHalf};

use crate::confirmation_tests::{receive, send};
use crate::skills;

type Input = BufReader<ReadHalf<DuplexStream>>;
type Output = WriteHalf<DuplexStream>;

struct Host {
    input: Input,
    output: Output,
    meta: Option<Value>,
    next: u64,
}

impl Host {
    async fn request(&mut self, method: &str, mut params: Value) -> Value {
        if let Some(meta) = &self.meta {
            params["_meta"] = meta.clone();
        }
        self.next += 1;
        send(
            &mut self.output,
            json!({"jsonrpc":"2.0","id":self.next,"method":method,
                "params":params}),
        )
        .await;
        let response = receive(&mut self.input).await;
        assert_eq!(response["id"], self.next, "{response}");
        response
    }
}

fn connect() -> (Host, tokio::task::JoinHandle<()>) {
    use rmcp::ServiceExt as _;
    let server = crate::server::Server::new(crate::Session::detached(
        std::env::temp_dir().join("pwrdisk-no-window.sock"),
        None,
    ));
    let (client, server_io) = tokio::io::duplex(1 << 20);
    let task = tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("server");
        let _ = running.waiting().await;
    });
    let (input, output) = tokio::io::split(client);
    let host = Host {
        input: BufReader::new(input),
        output,
        meta: None,
        next: 0,
    };
    (host, task)
}

/// The extension requires `resultType`, `ttlMs` and `cacheScope` on every
/// list and get result; rmcp passes custom results through untouched, so
/// nothing fills them in for us.
fn assert_complete(result: &Value) {
    assert_eq!(result["resultType"], "complete", "{result}");
    assert!(result["ttlMs"].is_u64(), "{result}");
    assert!(
        matches!(result["cacheScope"].as_str(), Some("public" | "private")),
        "{result}"
    );
}

/// The host side of the extension, from a declared server to verified
/// files.
async fn discover_and_load(host: &mut Host, capabilities: &Value) {
    // Only a server that declares both may be asked for skills.
    assert!(capabilities["resources"].is_object(), "{capabilities}");
    assert!(
        capabilities["extensions"][skills::EXTENSION].is_object(),
        "{capabilities}"
    );

    let listed = host.request("skills/list", json!({})).await;
    let listed = &listed["result"];
    assert_complete(listed);
    assert!(listed.get("nextCursor").is_none_or(Value::is_null));
    let entries = listed["skills"].as_array().expect("skills");
    assert_eq!(entries.len(), skills::SKILLS.len());

    for entry in entries {
        let address = entry["uri"].as_str().expect("uri");
        let name = entry["frontmatter"]["name"].as_str().expect("name");
        assert!(
            !entry["frontmatter"]["description"]
                .as_str()
                .unwrap_or("")
                .is_empty()
        );
        // The skill path's final segment is the frontmatter name.
        assert_eq!(address, format!("skill://{name}/SKILL.md"));

        // A host holding only the URI looks the skill up directly.
        let got = host.request("skills/get", json!({"uri": address})).await;
        let got = &got["result"];
        assert_complete(got);
        assert_eq!(&got["skill"], entry);

        let files = entry["resources"].as_array().expect("a file manifest");
        assert!(files.iter().any(|file| file["uri"] == address));
        for file in files {
            let uri = file["uri"].as_str().expect("file uri");
            let read =
                host.request("resources/read", json!({"uri": uri})).await;
            let contents = &read["result"]["contents"][0];
            assert_eq!(contents["uri"], uri);
            let text = contents["text"].as_str().expect("text");
            assert_eq!(file["size"], text.len(), "{uri}");
            assert_eq!(file["digest"], skills::digest(text), "{uri}");
            if uri == address {
                let frontmatter = Value::Object(skills::frontmatter(text));
                assert_eq!(entry["frontmatter"], frontmatter, "{uri}");
            }
        }
    }

    // An unknown skill is Invalid params, as for an unknown resource.
    let missing = host
        .request("skills/get", json!({"uri": "skill://nope/SKILL.md"}))
        .await;
    assert_eq!(missing["error"]["code"], -32602, "{missing}");
}

#[tokio::test]
async fn a_modern_host_discovers_and_loads_every_skill() {
    let (mut host, task) = connect();
    host.meta = Some(json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo":
            {"name": "skills-host", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    }));
    let discovered = host.request("server/discover", json!({})).await;
    let capabilities = discovered["result"]["capabilities"].clone();
    discover_and_load(&mut host, &capabilities).await;
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn an_initializing_host_discovers_and_loads_every_skill() {
    let (mut host, task) = connect();
    let info = host
        .request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "skills-host", "version": "1"}}),
        )
        .await;
    send(
        &mut host.output,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let capabilities = info["result"]["capabilities"].clone();
    discover_and_load(&mut host, &capabilities).await;
    task.abort();
    let _ = task.await;
}
