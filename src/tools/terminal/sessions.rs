use super::{
    command::{self, CommandSpec, OUTPUT_LIMIT},
    *,
};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
use tokio::sync::mpsc;
#[derive(Default)]
pub(super) struct Sessions {
    records: Mutex<BTreeMap<String, Arc<Session>>>,
}
struct Session {
    input: mpsc::Sender<Option<String>>,
    stop: watch::Sender<bool>,
    state: Mutex<State>,
    arguments: Vec<String>,
}
#[derive(Default)]
struct State {
    output: String,
    result: Option<Result<Value>>,
}
impl Drop for Sessions {
    fn drop(&mut self) {
        for session in self.records.get_mut().unwrap().values() {
            session.stop.send_replace(true);
        }
    }
}
impl Sessions {
    pub fn require(&self, id: &str) -> Result<()> {
        self.record(id).map(|_| ())
    }
    fn record(&self, id: &str) -> Result<Arc<Session>> {
        self.records
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("Unknown or expired session in this task".into())
    }
    pub async fn start(
        &self,
        spec: CommandSpec,
        w: &Workspace,
        seconds: u64,
        yield_ms: u64,
        cancel: watch::Receiver<bool>,
    ) -> Result<Value> {
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        let workspace = Workspace::new(&w.root)?;
        let id = crate::types::id();
        let (input, rx) = mpsc::channel(4);
        let (stop, mut stopped) = watch::channel(false);
        let session = Arc::new(Session {
            input,
            stop,
            state: Default::default(),
            arguments: spec.args.clone(),
        });
        {
            let mut records = self.records.lock().unwrap();
            if records.len() >= 32
                || records
                    .values()
                    .filter(|s| s.state.lock().unwrap().result.is_none())
                    .count()
                    >= 4
            {
                return Err("Task command session limit reached".into());
            }
            records.insert(id.clone(), session.clone());
        }
        let poll_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut output = |chunk: Value| {
                let mut state = session.state.lock().unwrap();
                let available = OUTPUT_LIMIT.saturating_sub(state.output.len());
                if available > 0 {
                    state
                        .output
                        .push_str(&bounded(chunk["text"].as_str().unwrap_or(""), available));
                }
            };
            let result = tokio::select! {biased;_=stopped.changed()=>Err("Cancelled".into()),result=command::run_interactive(&spec,&workspace,seconds,cancel,rx,&mut output)=>result};
            session.state.lock().unwrap().result = Some(result);
        });
        self.interact(&id, String::new(), false, yield_ms, poll_cancel)
            .await
    }
    pub async fn interact(
        &self,
        id: &str,
        chars: String,
        close: bool,
        yield_ms: u64,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Value> {
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        let session = self.record(id)?;
        if !chars.is_empty() {
            session
                .input
                .try_send(Some(chars))
                .map_err(|_| "Command input closed or busy")?;
        }
        if close {
            session
                .input
                .try_send(None)
                .map_err(|_| "Command input closed or busy")?;
        }
        if yield_ms > 0 {
            let end = tokio::time::Instant::now() + Duration::from_millis(yield_ms);
            while session.state.lock().unwrap().result.is_none()
                && tokio::time::Instant::now() < end
            {
                tokio::select! {biased;_=cancel.changed()=>return Err("Cancelled".into()),_=tokio::time::sleep(Duration::from_millis(10))=>{}}
            }
        }
        let mut state = session.state.lock().unwrap();
        let output = std::mem::take(&mut state.output);
        match &state.result {
            Some(Ok(result)) => {
                let mut result = result.clone();
                result["sessionId"] = json!(id);
                result["running"] = json!(false);
                result["output"] = json!(output);
                result["arguments"] = json!(session.arguments);
                Ok(result)
            }
            Some(Err(error)) => Err(error.clone()),
            None => Ok(json!({"ok":true,"sessionId":id,"running":true,"output":output})),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn fixture(body: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("lorca-session-{}", crate::types::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname=\"lorca_session_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), body).unwrap();
        root
    }
    #[tokio::test]
    async fn sessions_poll_and_accept_approved_stdin_and_report_exit() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = fixture(
            "#[test] fn input(){let mut s=String::new();std::io::stdin().read_line(&mut s).unwrap();assert_eq!(s.trim(),\"hello\");}",
        );
        let w = Workspace::new(&root).unwrap();
        let r = Registry::default();
        let l = Limits::default();
        let (_tx, c) = watch::channel(false);
        let exec = r.get("exec_command").unwrap();
        let p = exec
            .prepare(
                &w,
                json!({"executable":"cargo","arguments":["test"],"yield_time_ms":0}),
                &l,
            )
            .unwrap();
        assert!(p.approval);
        let v = exec
            .execute(&w, &p, &l, c.clone(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(v["running"], true);
        let id = v["sessionId"].as_str().unwrap();
        let input = r.get("write_stdin").unwrap();
        let p = input
            .prepare(
                &w,
                json!({"session_id":id,"chars":"hello\n","close_stdin":true}),
                &l,
            )
            .unwrap();
        assert!(p.approval);
        input
            .execute(&w, &p, &l, c.clone(), &mut |_| {})
            .await
            .unwrap();
        let final_result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let p = input
                    .prepare(&w, json!({"session_id":id,"yield_time_ms":1000}), &l)
                    .unwrap();
                assert!(!p.approval);
                let v = input
                    .execute(&w, &p, &l, c.clone(), &mut |_| {})
                    .await
                    .unwrap();
                if v["running"] == false {
                    break v;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(final_result["exitCode"], 0, "{final_result}");
        assert_eq!(final_result["arguments"], json!(["test"]));
        assert!(
            Registry::default()
                .get("write_stdin")
                .unwrap()
                .prepare(&w, json!({"session_id":id}), &l)
                .is_err()
        );
        drop(r);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn sessions_cancel_and_expire_with_task_registry() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = fixture(
            "#[test] fn wait(){std::thread::sleep(std::time::Duration::from_secs(20));std::fs::write(\"late\",\"bad\").unwrap();}",
        );
        let w = Workspace::new(&root).unwrap();
        let sessions = Sessions::default();
        let spec = CommandSpec::validate("cargo", &["test".into()], ".", &w).unwrap();
        let (tx, c) = watch::channel(false);
        let v = sessions.start(spec, &w, 30, 0, c.clone()).await.unwrap();
        let id = v["sessionId"].as_str().unwrap().to_string();
        let record = sessions.record(&id).unwrap();
        tx.send_replace(true);
        tokio::time::timeout(Duration::from_secs(5), async {
            while record.state.lock().unwrap().result.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            record
                .state
                .lock()
                .unwrap()
                .result
                .as_ref()
                .unwrap()
                .is_err()
        );
        drop(sessions);
        assert!(*record.stop.borrow());
        assert!(!root.join("late").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod drop_tests {
    use super::*;
    #[tokio::test]
    async fn dropping_task_registry_stops_a_running_session() {
        if std::env::var_os("LORCA_COMMAND_SANDBOX").is_some() {
            return;
        }
        let root = std::env::temp_dir().join(format!("lorca-session-drop-{}", crate::types::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname=\"session_drop_fixture\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "#[test] fn wait(){std::thread::sleep(std::time::Duration::from_secs(20));}",
        )
        .unwrap();
        let w = Workspace::new(&root).unwrap();
        let sessions = Sessions::default();
        let spec = CommandSpec::validate("cargo", &["test".into()], ".", &w).unwrap();
        let (_tx, c) = watch::channel(false);
        let v = sessions.start(spec, &w, 30, 0, c).await.unwrap();
        let record = sessions.record(v["sessionId"].as_str().unwrap()).unwrap();
        drop(sessions);
        tokio::time::timeout(Duration::from_secs(5), async {
            while record.state.lock().unwrap().result.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            record
                .state
                .lock()
                .unwrap()
                .result
                .as_ref()
                .unwrap()
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
