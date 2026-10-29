//! Behavioral contracts for B-E; real filesystem/process effects in disposable roots.
use lorca::agent::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lorca-contract-{}", lorca::types::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Script(Mutex<VecDeque<Turn>>);
impl Inference for Script {
    fn interprets_tasks(&self) -> bool {
        true
    }
    fn capacity(&self) -> FutureResult<'_, Option<u64>> {
        Box::pin(async { Ok(Some(32768)) })
    }
    fn generate<'a>(&'a self, _: &'a [Value], _: &'a [Value]) -> FutureResult<'a, Turn> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .ok_or("Script exhausted".into())
        })
    }
}
fn turn(name: &str, args: Value) -> Turn {
    let call = ToolCall {
        id: lorca::types::id(),
        name: name.into(),
        arguments: args,
    };
    Turn {
        content: String::new(),
        calls: vec![call.clone()],
        message: json!({"role":"assistant","tool_calls":[{"id":call.id,"function":{"name":name,"arguments":call.arguments}}]}),
        finish_reason: FinishReason::Stop,
    }
}
fn answer(text: &str) -> Turn {
    Turn {
        content: text.into(),
        calls: vec![],
        message: json!({"role":"assistant","content":text}),
        finish_reason: FinishReason::Stop,
    }
}
fn intent(kind: &str, targets: Value) -> Turn {
    turn(
        "interpret_task",
        json!({"relation":"new","objective":"Perform the requested bounded operation and report its actual result","outcomes":[{"kind":kind,"target":targets.as_array().and_then(|a|a.first()).cloned().unwrap_or(Value::Null)}],"edit_targets":targets,"forbidden_targets":[],"read_only":false,"commands_forbidden":false,"clarification":null}),
    )
}
async fn run(task: &str, root: &Temp, turns: Vec<Turn>) -> TaskView {
    let agent = Arc::new(Agent::default());
    let id = agent
        .start(
            "test",
            task,
            &root.0,
            Limits {
                iterations: 5,
                command_seconds: 5,
                ..Default::default()
            },
            Arc::new(Script(Mutex::new(turns.into()))),
            Registry::default(),
        )
        .unwrap();
    let mut seen = std::collections::HashSet::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let view = agent.snapshot().unwrap();
            if let Some(a) = &view.approval
                && seen.insert(a.id.clone())
            {
                agent
                    .decide("test", &id, &a.id, Decision::AllowOnce)
                    .unwrap();
            }
            if !view.status.active() {
                return view;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn scoped_negative_request_allows_only_its_target() {
    let root = Temp::new();
    std::fs::write(root.0.join("one.rs"), "before").unwrap();
    std::fs::write(root.0.join("two.rs"), "untouched").unwrap();
    let v = run(
        "Fix one.rs; do not edit other source files.",
        &root,
        vec![
            intent("edit", json!(["one.rs"])),
            turn(
                "apply_patch",
                json!({"path":"one.rs","old_text":"before","new_text":"after"}),
            ),
            answer("Changed one.rs."),
        ],
    )
    .await;
    assert_eq!(v.status, Status::Completed, "{:?}", v.result);
    assert_eq!(
        std::fs::read_to_string(root.0.join("one.rs")).unwrap(),
        "after"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("two.rs")).unwrap(),
        "untouched"
    );
}
#[tokio::test]
async fn model_interpretation_cannot_erase_read_only_or_expand_targets() {
    for (task, path) in [
        ("Inspect one.rs; do not edit any files.", "one.rs"),
        ("Fix one.rs; do not edit other files.", "two.rs"),
    ] {
        let root = Temp::new();
        for n in ["one.rs", "two.rs"] {
            std::fs::write(root.0.join(n), "before").unwrap();
        }
        let v = run(
            task,
            &root,
            vec![
                intent("edit", json!(["one.rs"])),
                turn(
                    "apply_patch",
                    json!({"path":path,"old_text":"before","new_text":"bad"}),
                ),
                answer("Could not edit."),
            ],
        )
        .await;
        assert_ne!(v.status, Status::Completed);
        for n in ["one.rs", "two.rs"] {
            assert_eq!(std::fs::read_to_string(root.0.join(n)).unwrap(), "before");
        }
    }
}
#[tokio::test]
async fn semantic_edit_without_old_keywords_cannot_complete_without_evidence() {
    let root = Temp::new();
    let v = run(
        "Make the gradient control change hue across the rainbow",
        &root,
        vec![
            intent("edit", json!([])),
            answer("Done."),
            answer("Done."),
            answer("Done."),
        ],
    )
    .await;
    assert_ne!(v.status, Status::Completed);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}
#[tokio::test]
async fn related_plain_text_questions_enter_a_real_waiting_state() {
    let root = Temp::new();
    let v = run(
        "Create a file",
        &root,
        vec![
            intent("edit", json!([])),
            answer("Which filename should I use? What contents should it contain?"),
        ],
    )
    .await;
    assert_eq!(v.status, Status::WaitingForInput);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[tokio::test]
async fn waiting_for_filename_does_not_make_creation_read_only() {
    let root = Temp::new();
    let mut mistaken = intent("edit", json!([]));
    mistaken.calls[0].arguments["read_only"] = json!(true);
    let v = run(
        "I need to create a file. Before editing, use request_user_input to ask me which filename and contents to use. Wait for my answer.",
        &root,
        vec![
            mistaken,
            intent("edit", json!([])),
            turn("request_user_input", json!({"question":"Which filename and contents should I use?"})),
        ],
    ).await;
    assert_eq!(v.status, Status::WaitingForInput, "{:?}", v.result);
    let contract = v.contract.expect("persist the corrected task contract");
    assert_eq!(contract["authority"]["scope"]["read_only"], json!(false));
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[tokio::test]
async fn abstract_repository_inspection_does_not_require_a_file_named_repository() {
    let root = Temp::new();
    std::fs::write(root.0.join("README.md"), "real project contents").unwrap();
    let v = run(
        "Inspect this repository and report back. Do not edit files or run commands.",
        &root,
        vec![
            intent("inspect", json!(["repository"])),
            turn("list_directory", json!({"path":"."})),
            turn("read_file", json!({"path":"README.md"})),
            answer("The repository contains README.md describing the project."),
        ],
    )
    .await;
    assert_eq!(v.status, Status::Completed, "{:?}", v.result);
    assert_eq!(
        std::fs::read_to_string(root.0.join("README.md")).unwrap(),
        "real project contents"
    );
}
