//! Replacement runtime contract checks: disposable storage and a local mock model only.
use axum::{
    Json, Router,
    routing::{get, post},
};
use lorca::{server::Server, storage::Repository, types::id};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-runtime-{}", id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn wait(server: &Server, status: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(v) = server.agent().snapshot() {
                if v["status"] == status {
                    return v;
                }
                if v["status"] == "failed" {
                    panic!("runtime failed: {}", v["result"]);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("task did not reach expected state")
}
async fn exercise(llama: bool, mode: &str) {
    let temp = Temp::new();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let roster_mode = mode == "roster_allow_once";
    let decision_mode = if roster_mode { "allow_once" } else { mode };
    let compact = mode.starts_with("compact");
    let compact_fail = mode == "compact_fail";
    let chat_only = mode == "chat" || compact;
    let repeat_denied = mode == "repeat_denied";
    let app=Router::new()
        .route("/api/tags",get(||async{Json(json!({"models":[{"name":"mock"}]}))}))
        .route("/api/show",post(||async{Json(json!({"capabilities":["tools","thinking"]}))}))
        .route("/api/ps",get(||async{Json(json!({"models":[{"name":"mock","context_length":8192}]}))}))
        .route("/v1/models",get(||async{Json(json!({"data":[{"id":"mock","context_length":8192}]}))}))
        .route("/props",get(||async{Json(json!({"n_ctx":8192}))}))
        .route(if llama{"/v1/chat/completions"}else{"/api/chat"},post(move|Json(body):Json<Value>|{
            let count=count.clone();async move{
                if body["messages"].as_array().is_none_or(|a|a.is_empty()){return "{\"done\":true}".to_owned()}
                count.fetch_add(1,Ordering::SeqCst);
                if compact && body["tools"].as_array().is_some_and(Vec::is_empty) {
                    if compact_fail {return format!("{}\n",json!({"error":"checkpoint fixture unavailable","done":true}));}
                    if llama {return format!("data: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"delta":{"content":"Objective: finish the current task. Earlier evidence is saved; retrieve it when needed."},"finish_reason":"stop"}]}));}
                    return format!("{}\n",json!({"message":{"role":"assistant","content":"Objective: finish the current task. Earlier evidence is saved; retrieve it when needed."},"done":true}));
                }
                let last=body["messages"].as_array().unwrap().last().unwrap();
                let guidance=repeat_denied && body["tools"].as_array().is_some_and(|a|a.len()==1 && a[0]["function"]["name"]=="request_user_input");
                let done=chat_only||guidance||(!repeat_denied && last["role"]=="tool");
                let args=json!({"path":"created.txt","old_text":"","new_text":"hello"});
                if llama {
                    let delta=if done{json!({"content":"Finished from actual tool results.","reasoning_content":if chat_only{"Fixture reasoning"}else{""}})}else{json!({"tool_calls":[{"index":0,"id":"call1","function":{"name":"apply_patch","arguments":args.to_string()}}]})};
                    format!("data: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"delta":delta,"finish_reason":if done{"stop"}else{"tool_calls"}}]}))
                }else{
                    let msg=if done{json!({"role":"assistant","content":"Finished from actual tool results.","thinking":if chat_only{"Fixture reasoning"}else{""}})}else{json!({"role":"assistant","content":"","tool_calls":[{"function":{"name":"apply_patch","arguments":args}}]})};
                    format!("{}\n",json!({"message":msg,"done":true,"done_reason":"stop","prompt_eval_count":200,"eval_count":20,"eval_duration":1000000000}))
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let data = temp.0.join("data");
    let work = temp.0.join("work");
    std::fs::create_dir(&work).unwrap();
    let repository = Repository::new(data.clone()).unwrap();
    if compact {
        let mut chat = lorca::types::Conversation::default();
        for _ in 0..24 {
            chat.messages.push(lorca::types::Message::new(
                "user",
                "Earlier source material. ".repeat(35),
            ));
            chat.messages.push(lorca::types::Message::new(
                "assistant",
                "Earlier verified response.".into(),
            ));
        }
        repository.save(&chat).unwrap();
    }
    let server = Server::new(repository).unwrap();
    let connected=server.command(json!({"op":"connect","host":host,"provider":if llama{"llama_cpp"}else{"ollama"},"model":"mock"})).await.unwrap();
    let owner = connected["session"].as_str().unwrap();
    let cid = connected["state"]["sessions"][0]["selectedId"]
        .as_str()
        .unwrap();
    let request = json!({"op":"agent_start","session":owner,"conversationId":cid,"requestId":id(),"task":if chat_only{"hi"}else{"Create the file"},"workspace":work,"model":"mock","effort":if chat_only{"nominal"}else{"off"}});
    let request = if roster_mode {
        let tid = id();
        let add =
            json!({"op":"roster_add","session":owner,"taskId":tid,"objective":"Create the file"});
        server.command(add.clone()).await.unwrap();
        server.command(add).await.unwrap();
        assert_eq!(
            server.snapshot().roster.tasks.len(),
            1,
            "saving a task replayed"
        );
        assert_eq!(
            server.snapshot().conversations[0].messages.len(),
            0,
            "saving roster work changed ordinary chat"
        );
        let reopened = Server::new(Repository::new(data.clone()).unwrap()).unwrap();
        assert_eq!(reopened.snapshot().roster.tasks[0].status, "queued");
        assert!(reopened.agent().snapshot().is_none());
        json!({"op":"roster_run","session":owner,"taskId":tid,"workspace":work,"model":"mock"})
    } else {
        request
    };
    let started = server.command(request.clone()).await.unwrap();
    if chat_only {
        let task = wait(&server, "completed").await;
        server.checkpoint().unwrap();
        let snapshot = server.snapshot();
        let messages = &snapshot.conversations[0].messages;
        assert_eq!(messages.len(), if compact { 50 } else { 2 });
        let messages = &messages[messages.len() - 2..];
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[0].content, "hi",
            "assistant output overwrote user input"
        );
        assert!(!messages[0].extra.contains_key("agentHistory"));
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Finished from actual tool results.");
        assert_eq!(
            messages[1].extra["agentKind"], "result",
            "UI would not render reasoning without result marker"
        );
        assert!(
            task["thinking"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["state"] == "complete" && b["text"] == "Fixture reasoning")
        );
        assert!(!messages[1].content.contains("Fixture reasoning"));
        let reopened = Server::new(Repository::new(data).unwrap()).unwrap();
        assert_eq!(
            reopened.snapshot().conversations[0].messages[if compact { 48 } else { 0 }].content,
            "hi"
        );
        if !compact {
            assert_eq!(requests.load(Ordering::SeqCst), 1);
        } else {
            assert!(requests.load(Ordering::SeqCst) > 1);
        }
        if compact {
            let chat = &reopened.snapshot().conversations[0];
            let working = reopened.repository.working_state(&chat.id).unwrap();
            if compact_fail {
                assert!(working.state.is_empty());
            } else {
                assert!(working.covered > 0);
                assert!(working.state.contains("Objective"));
            }
            assert_eq!(
                chat.messages.len(),
                50,
                "compaction deleted original records"
            );
            let journal = reopened.repository.journal_directory(chat).unwrap();
            let markdown = std::fs::read_to_string(journal.join("journal.md")).unwrap();
            assert!(markdown.contains("Earlier source material"));
            assert!(
                !markdown.contains("Fixture reasoning"),
                "private reasoning entered the readable journal"
            );
            assert!(journal.join("checkpoint.md").exists());
            let connected=reopened.command(json!({"op":"connect","host":host,"provider":if llama{"llama_cpp"}else{"ollama"},"model":"mock"})).await.unwrap();
            let owner = connected["session"].as_str().unwrap();
            reopened.command(json!({"op":"agent_start","session":owner,"conversationId":chat.id,"requestId":id(),"task":"Continue using the saved progress","workspace":work,"model":"mock","effort":"off"})).await.unwrap();
            wait(&reopened, "completed").await;
            assert_eq!(reopened.snapshot().conversations[0].messages.len(), 52);
        }
        mock.abort();
        return;
    }
    let approval = wait(&server, "awaiting_approval").await;
    assert!(
        !work.join("created.txt").exists(),
        "write executed before approval"
    );
    let event = approval["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "approval_required")
        .unwrap();
    assert_eq!(
        event["data"]["id"], approval["approval"]["id"],
        "UI approval event shape differs"
    );
    let replay = server.command(request).await.unwrap();
    if roster_mode {
        assert_eq!(
            server.snapshot().roster.tasks[0].run_id.as_deref(),
            approval["id"].as_str()
        );
        assert_eq!(
            started["roster"]["tasks"][0]["conversationId"],
            replay["roster"]["tasks"][0]["conversationId"]
        );
        assert_eq!(
            server.snapshot().conversations.len(),
            2,
            "replayed roster run created another chat"
        );
        let reopened = Server::new(Repository::new(data.clone()).unwrap()).unwrap();
        assert_eq!(reopened.snapshot().roster.tasks[0].status, "incomplete");
        assert!(
            reopened.agent().snapshot().is_none(),
            "restart replayed a protected action"
        );
    } else {
        assert_eq!(started["taskId"], replay["taskId"]);
    }
    let foreign = server
        .command(json!({"op":"connect","host":host,"provider":if llama{"llama_cpp"}else{"ollama"}}))
        .await
        .unwrap();
    let mut decision = json!({"op":"agent_decide","session":foreign["session"],"taskId":approval["id"],"approvalId":approval["approval"]["id"],"decision":"allow_once"});
    assert!(
        server.command(decision.clone()).await.is_err(),
        "another session approved an action"
    );
    decision["session"] = json!(owner);
    decision["decision"] = json!(if repeat_denied { "deny" } else { decision_mode });
    server.command(decision.clone()).await.unwrap();
    wait(
        &server,
        if mode == "cancel" {
            "cancelled"
        } else if repeat_denied {
            "waiting_for_input"
        } else {
            "completed"
        },
    )
    .await;
    if decision_mode == "allow_once" {
        assert_eq!(
            std::fs::read_to_string(work.join("created.txt")).unwrap(),
            "hello"
        );
    } else {
        assert!(
            !work.join("created.txt").exists(),
            "denied or cancelled write executed"
        );
    }
    assert!(
        server.command(decision).await.is_err(),
        "approval was reused"
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        if mode == "cancel" {
            1
        } else if repeat_denied {
            4
        } else {
            2
        }
    );
    if repeat_denied {
        let task = server.agent().snapshot().unwrap();
        let events = task["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e["kind"] == "approval_required")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["kind"] == "tool_started")
                .count(),
            0
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["data"]["result"]["blockedRepeat"] == true)
                .count(),
            2
        );
    }
    let reopened = Server::new(Repository::new(data).unwrap()).unwrap();
    assert!(reopened.agent().snapshot().is_none());
    if roster_mode {
        assert_eq!(reopened.snapshot().roster.tasks[0].status, "completed");
        assert_eq!(
            reopened.snapshot().roster.tasks[0].run_id.as_deref(),
            approval["id"].as_str()
        );
        assert!(
            reopened
                .snapshot()
                .conversations
                .iter()
                .find(|c| c.id == cid)
                .unwrap()
                .messages
                .is_empty()
        );
    }
    if mode != "cancel" {
        assert!(
            reopened.snapshot().conversations[0]
                .messages
                .iter()
                .any(|m| m.content.contains("Finished"))
        );
    }
    if compact {
        let chat = &reopened.snapshot().conversations[0];
        let working = reopened.repository.working_state(&chat.id).unwrap();
        if compact_fail {
            assert!(working.state.is_empty());
        } else {
            assert!(working.covered > 0);
            assert!(working.state.contains("Objective"));
        }
        assert_eq!(
            chat.messages.len(),
            50,
            "compaction deleted original records"
        );
        let journal = reopened.repository.journal_directory(chat).unwrap();
        let markdown = std::fs::read_to_string(journal.join("journal.md")).unwrap();
        assert!(markdown.contains("Earlier source material"));
        assert!(
            !markdown.contains("Fixture reasoning"),
            "private reasoning entered the readable journal"
        );
        assert!(journal.join("checkpoint.md").exists());
    }
    mock.abort();
}
#[tokio::test]
async fn ollama_approval_execution_persistence() {
    exercise(false, "allow_once").await
}
#[tokio::test]
async fn llama_cpp_approval_execution_persistence() {
    exercise(true, "allow_once").await
}

#[tokio::test]
async fn denied_action_has_no_effect() {
    exercise(false, "deny").await
}
#[tokio::test]
async fn cancelled_approval_has_no_effect() {
    exercise(false, "cancel").await
}

#[tokio::test]
async fn ollama_chat_keeps_roles_and_reasoning_separate() {
    exercise(false, "chat").await
}
#[tokio::test]
async fn llama_cpp_chat_keeps_roles_and_reasoning_separate() {
    exercise(true, "chat").await
}

#[tokio::test]
async fn repeated_denial_reuses_feedback_then_requests_guidance() {
    exercise(false, "repeat_denied").await
}

#[tokio::test]
async fn checkpoints_survive_restart_without_deleting_history() {
    exercise(false, "compact").await;
}
#[tokio::test]
async fn llama_checkpoint_continuation() {
    exercise(true, "compact").await;
}
#[tokio::test]
async fn failed_compaction_keeps_originals_and_continues() {
    exercise(false, "compact_fail").await;
}

#[tokio::test]
async fn roster_local_run_preserves_chat_gates_and_survives_restart() {
    exercise(false, "roster_allow_once").await;
}

#[test]
fn unreadable_roster_does_not_break_chat_or_overwrite_originals() {
    let temp = Temp::new();
    let repository = Repository::new(temp.0.clone()).unwrap();
    let path = temp.0.join("roster.json");
    std::fs::write(&path, b"not valid json").unwrap();
    let server = Server::new(repository).unwrap();
    assert_eq!(server.snapshot().conversations.len(), 1);
    assert!(server.snapshot().roster.error.is_some());
    assert!(server.snapshot().roster.save(&temp.0).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"not valid json");
}
