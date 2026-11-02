//! The relay must neither freeze discovery nor translate away MCP fields.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::confirmation_tests::{receive, send};

#[tokio::test]
async fn an_older_relay_without_a_path_base_rejects_relative_tool_paths() {
    let temp = tempfile::tempdir().expect("fixture");
    let (client, desktop_io) = tokio::io::duplex(65_536);
    let task = tokio::spawn(crate::desktop::serve_connection(
        Box::new(desktop_io),
        temp.path().join("absent.sock"),
    ));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(
        &mut output,
        json!({"transport_version":1,"relay_version":"old"}),
    )
    .await;
    assert_eq!(receive(&mut input).await["transport_version"], 1);
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"legacy","version":"1"}
        }}),
    )
    .await;
    let info = receive(&mut input).await;
    assert!(
        info["result"]["_meta"]["com.pwrdrvr.pwrdisk/interface"]["pathBase"]
            .is_null()
    );
    assert!(
        info["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("relative paths are rejected")
    );
    send(
        &mut output,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    for (id, name, arguments) in [
        (2, "pwrdisk_scan", json!({"path":"."})),
        (3, "pwrdisk_ls", json!({"path":"."})),
        (4, "pwrdisk_find", json!({"path":"."})),
        (5, "pwrdisk_mark", json!({"paths":["junk"]})),
        (6, "pwrdisk_unmark", json!({"paths":["junk"]})),
        (7, "pwrdisk_show", json!({"path":"."})),
    ] {
        send(
            &mut output,
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
                "name":name,"arguments":arguments
            }}),
        )
        .await;
        let result = receive(&mut input).await;
        assert_eq!(result["result"]["isError"], true, "{name}: {result}");
        assert!(
            result["result"]["content"]
                .to_string()
                .contains("Use an absolute path"),
            "{name}: {result}"
        );
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn a_relative_transport_path_base_is_refused_before_initialization() {
    let (client, desktop_io) = tokio::io::duplex(4096);
    let task = tokio::spawn(crate::desktop::serve_connection(
        Box::new(desktop_io),
        "unused.sock".into(),
    ));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"transport_version":1,"relay_version":"new","working_directory":"."})).await;
    assert_eq!(receive(&mut input).await["transport_version"], 1);
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("must be absolute")
    );
    let mut line = String::new();
    assert_eq!(input.read_line(&mut line).await.expect("EOF"), 0);
}

#[tokio::test]
async fn a_mixed_release_relay_exposes_desktop_identity_schemas_and_skills() {
    let temp = tempfile::tempdir().expect("fixture");
    let command_path = temp.path().join("no-window.sock");
    let (client, relay_io) = tokio::io::duplex(65_536);
    let (upstream, desktop_io) = tokio::io::duplex(65_536);
    let desktop = tokio::spawn(crate::desktop::serve_connection(
        Box::new(desktop_io),
        command_path,
    ));
    let proxy = tokio::spawn(async move {
        let (input, output) = tokio::io::split(relay_io);
        crate::desktop::relay(Box::new(upstream), input, output, "0.12.0").await
    });
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18", "capabilities":{},
            "clientInfo":{"name":"future-host","version":"42"}
        }}),
    )
    .await;
    let initialized = receive(&mut input).await;
    let info = &initialized["result"];
    assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let ownership = &info["_meta"]["com.pwrdrvr.pwrdisk/interface"];
    assert_eq!(ownership["owner"], "desktop");
    assert_eq!(ownership["relayVersion"], "0.12.0");
    assert_eq!(ownership["desktopVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        ownership["pathBase"],
        json!(std::env::current_dir().expect("relay cwd"))
    );
    assert!(
        info["instructions"]
            .as_str()
            .unwrap()
            .contains("rediscover")
    );
    send(
        &mut output,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let listed = receive(&mut input).await;
    let tools = listed["result"]["tools"].as_array().expect("tools");
    for name in [
        "pwrdisk_scan",
        "pwrdisk_ls",
        "pwrdisk_find",
        "pwrdisk_mark",
        "pwrdisk_unmark",
        "pwrdisk_show",
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        let field = if name == "pwrdisk_mark" || name == "pwrdisk_unmark" {
            "paths"
        } else {
            "path"
        };
        assert!(
            tool["inputSchema"]["properties"][field]["description"]
                .as_str()
                .unwrap()
                .contains("session path base"),
            "{name}"
        );
    }
    let remove = tools
        .iter()
        .find(|tool| tool["name"] == "pwrdisk_remove")
        .unwrap();
    assert_eq!(remove["annotations"]["destructiveHint"], true);
    assert_eq!(remove["annotations"]["idempotentHint"], false);
    assert!(
        remove["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("preview_id"))
    );
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"pwrdisk_remove","arguments":{"preview_id":17,"unknown":true}
        }}),
    )
    .await;
    let invalid = receive(&mut input).await;
    assert!(
        invalid.get("error").is_some() || invalid["result"]["isError"] == true
    );
    for (id, method, params, field) in [
        (4, "skills/list", json!({}), "skills"),
        (
            5,
            "resources/read",
            json!({"uri":"skill://pwrdisk-free-space/SKILL.md"}),
            "contents",
        ),
        (6, "prompts/get", json!({"name":"free-space"}), "messages"),
    ] {
        send(
            &mut output,
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )
        .await;
        let response = receive(&mut input).await;
        assert!(response["result"][field].is_array(), "{response}");
    }
    proxy.abort();
    desktop.abort();
    let _ = proxy.await;
    let _ = desktop.await;
}

#[tokio::test]
async fn discovery_updates_and_bidirectional_metadata_pass_without_translation()
{
    // These messages intentionally contain future methods, schemas and
    // fields unknown to this build. A parsed forwarding router would lose
    // them, or refuse to call the newly discovered tool.
    let new_tool = json!({"name":"future_cleanup","inputSchema":{
        "type":"object","properties":{"ticket":{"type":"string"}},
        "required":["ticket"],"additionalProperties":false
    },"annotations":{"destructiveHint":true,"readOnlyHint":false},
        "_meta":{"future.example/policy":{"revision":2}}});
    let steps: Vec<(bool, Value)> = vec![
        (true, json!({"id":1,"method":"tools/list"})),
        (false, json!({"id":1,"result":{"tools":[]}})),
        (
            false,
            json!({"method":"notifications/tools/list_changed","params":{"_meta":{"generation":2}}}),
        ),
        (
            false,
            json!({"method":"notifications/prompts/list_changed"}),
        ),
        (
            false,
            json!({"method":"notifications/resources/list_changed"}),
        ),
        (
            false,
            json!({"method":"notifications/resources/updated","params":{"uri":"skill://future/SKILL.md"}}),
        ),
        (true, json!({"id":2,"method":"tools/list"})),
        (false, json!({"id":2,"result":{"tools":[new_tool]}})),
        (
            true,
            json!({"id":3,"method":"resources/read","params":{"uri":"skill://future/SKILL.md"}}),
        ),
        (
            false,
            json!({"id":3,"result":{"contents":[{"uri":"skill://future/SKILL.md","text":"updated safety brief"}]}}),
        ),
        (
            true,
            json!({"id":4,"method":"tools/call","params":{
                "name":"future_cleanup","arguments":{"ticket":"exact-operation"},
                "_meta":{"progressToken":"42","host.example/approval":{"revision":2}}
            }}),
        ),
        (
            false,
            json!({"id":"desktop-confirm","method":"elicitation/create","params":{
                "message":"Confirm exact-operation","requestedSchema":{"type":"object"},
                "_meta":{"future.example/confirmation":"single-use"}
            }}),
        ),
        (
            true,
            json!({"id":"desktop-confirm","result":{"action":"accept","content":{"confirmed":true}}}),
        ),
        (
            false,
            json!({"id":4,"result":{"content":[],"_meta":{"future.example/executed":2}}}),
        ),
    ];
    let expected = steps.clone();
    let (client, relay_io) = tokio::io::duplex(65_536);
    let (upstream, desktop_io) = tokio::io::duplex(65_536);
    let desktop = tokio::spawn(async move {
        let (input, mut output) = tokio::io::split(desktop_io);
        let mut input = BufReader::new(input);
        let hello = receive(&mut input).await;
        assert_eq!(hello["relay_version"], "0.12.0");
        // A newer release needs only the same stream contract.
        send(
            &mut output,
            json!({"transport_version":1,"desktop_version":"99.0.0"}),
        )
        .await;
        for (from_host, message) in steps {
            if from_host {
                assert_eq!(receive(&mut input).await, message);
            } else {
                send(&mut output, message).await;
            }
        }
    });
    let proxy = tokio::spawn(async move {
        let (input, output) = tokio::io::split(relay_io);
        crate::desktop::relay(Box::new(upstream), input, output, "0.12.0").await
    });
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    for (from_host, message) in expected {
        if from_host {
            send(&mut output, message).await;
        } else {
            assert_eq!(receive(&mut input).await, message);
        }
    }
    desktop.await.expect("desktop");
    proxy.await.expect("proxy").expect("stream");
}

#[tokio::test]
async fn a_stream_protocol_mismatch_fails_before_mcp_is_forwarded() {
    let (upstream, desktop_io) = tokio::io::duplex(4096);
    let desktop = tokio::spawn(async move {
        let mut io = BufReader::new(desktop_io);
        let _ = receive(&mut io).await;
        io.get_mut()
            .write_all(
                b"{\"transport_version\":99,\"desktop_version\":\"0.14.0\"}\n",
            )
            .await
            .expect("ready");
        let mut rest = String::new();
        assert_eq!(io.read_line(&mut rest).await.expect("EOF"), 0);
    });
    let error = crate::desktop::relay(
        Box::new(upstream),
        b"private MCP data".as_slice(),
        tokio::io::sink(),
        "0.14.0",
    )
    .await
    .expect_err("incompatible transport");
    assert!(error.to_string().contains("stream protocols"));
    assert!(error.to_string().contains("restart/reconnect"));
    desktop.await.expect("desktop");
}

#[tokio::test]
async fn modern_discovery_and_calls_report_desktop_authority_and_client_identity()
 {
    use rmcp::ServiceExt as _;
    let temp = tempfile::tempdir().expect("fixture");
    let server = crate::server::Server::desktop(
        crate::Session::desktop(temp.path().join("absent.sock")),
        "older-relay".into(),
    );
    let (client, server_io) = tokio::io::duplex(65_536);
    let task = tokio::spawn(async move {
        server
            .serve(server_io)
            .await
            .expect("server")
            .waiting()
            .await
            .expect("connection")
    });
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"modern-host","version":"2"},
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":meta}})).await;
    let info = receive(&mut input).await;
    assert_eq!(
        info["result"]["_meta"]["com.pwrdrvr.pwrdisk/interface"]["owner"],
        "desktop"
    );
    assert_eq!(info["result"]["ttlMs"], 0);
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "_meta":meta,"name":"pwrdisk_status","arguments":{}
        }}),
    )
    .await;
    let status = receive(&mut input).await;
    let text = status["result"]["content"].to_string();
    assert!(text.contains("modern-host"), "{status}");
    assert!(text.contains("older-relay"), "{status}");
    task.abort();
    let _ = task.await;
}
