use super::{Registry, Workspace, types::*};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::{oneshot, watch};

pub(super) struct State {
    pub(super) view: Option<TaskView>,
    cancel: watch::Sender<bool>,
    pending: Option<oneshot::Sender<Decision>>,
}
pub struct Agent {
    pub(super) state: Mutex<State>,
    changes: watch::Sender<u64>,
}
impl Default for Agent {
    fn default() -> Self {
        let (cancel, _) = watch::channel(false);
        let (changes, _) = watch::channel(0);
        Self {
            state: Mutex::new(State {
                view: None,
                cancel,
                pending: None,
            }),
            changes,
        }
    }
}
impl Agent {
    pub fn snapshot(&self) -> Option<TaskView> {
        self.state.lock().unwrap().view.clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    pub(super) fn changed(&self) {
        if let Some(view) = self.state.lock().unwrap().view.as_mut() {
            view.revision += 1;
        }
        self.changes.send_modify(|v| *v += 1);
    }
    pub fn start(
        self: &Arc<Self>,
        owner: &str,
        task: &str,
        workspace: &Path,
        limits: Limits,
        inference: Arc<dyn Inference>,
        registry: Registry,
    ) -> Result<String> {
        self.submit(
            Submission {
                owner: owner.into(),
                model: None,
                task: task.into(),
                workspace: workspace.into(),
                limits,
                conversation_id: None,
                request_id: None,
                history: vec![],
            },
            inference,
            registry,
        )
    }
    pub fn submit(
        self: &Arc<Self>,
        request: Submission,
        inference: Arc<dyn Inference>,
        registry: Registry,
    ) -> Result<String> {
        let Submission {
            owner,
            model,
            task,
            workspace,
            limits,
            conversation_id,
            request_id,
            history,
        } = request;
        limits.validate()?;
        if task.trim().is_empty() || task.len() > 8000 {
            return Err("Task must contain 1–8000 bytes".into());
        }
        let workspace = if workspace.as_os_str().is_empty() {
            None
        } else {
            Some(Workspace::new(&workspace)?)
        };
        let mut s = self.state.lock().unwrap();
        if s.view.as_ref().is_some_and(|v| v.status.active()) {
            return Err("An agent task is already active".into());
        }
        let id = crate::types::id();
        let (cancel, rx) = watch::channel(false);
        s.cancel = cancel;
        s.pending = None;
        s.view = Some(TaskView {
            revision: 0,
            conversation_id,
            request_id,
            id: id.clone(),
            owner,
            model,
            task: task.clone(),
            workspace: workspace
                .as_ref()
                .map(|w| w.root.display().to_string())
                .unwrap_or_default(),
            status: Status::Running,
            events: vec![],
            approval: None,
            result: None,
            assistant_text: String::new(),
            usage: Default::default(),
            generating: false,
            transcript: vec![],
            audit: vec![],
        });
        drop(s);
        self.emit(
            "task_started",
            json!({"task":task,"workspace":workspace.as_ref().map(|w|&w.root),"limits":limits}),
        );
        let this = self.clone();
        tokio::spawn(async move {
            let result = this
                .run(
                    workspace.as_ref(),
                    &limits,
                    inference,
                    registry,
                    rx.clone(),
                    history,
                )
                .await;
            let (status, kind, result) = if *rx.borrow() {
                (
                    Status::Cancelled,
                    "task_cancelled",
                    "Task cancelled; no pending approvals will execute".into(),
                )
            } else {
                match result {
                    Ok(text) => (Status::Completed, "task_completed", text),
                    Err(e) => (Status::Failed, "task_failed", e),
                }
            };
            let mut s = this.state.lock().unwrap();
            let view = s.view.as_mut().unwrap();
            view.status = status;
            view.generating = false;
            view.result = Some(result.clone());
            view.approval = None;
            append_event(view, kind, json!({"result":result}));
            s.pending = None;
            drop(s);
            this.changed();
        });
        Ok(id)
    }
    pub fn cancel(&self, owner: &str, task_id: &str) -> Result<()> {
        let s = self.state.lock().unwrap();
        let v = s.view.as_ref().ok_or("No agent task")?;
        if v.owner != owner || v.id != task_id {
            return Err("Task does not belong to this session".into());
        }
        if v.status.active() {
            s.cancel.send_replace(true);
        }
        Ok(())
    }
    pub fn cancel_owner(&self, owner: &str) {
        let s = self.state.lock().unwrap();
        if s.view
            .as_ref()
            .is_some_and(|v| v.owner == owner && v.status.active())
        {
            s.cancel.send_replace(true);
        }
    }
    pub fn decide(
        &self,
        owner: &str,
        task: &str,
        approval: &str,
        decision: Decision,
    ) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let v = s.view.as_ref().ok_or("No task")?;
        if v.id != task
            || v.owner != owner
            || v.approval.as_ref().is_none_or(|a| a.id != approval)
            || *s.cancel.borrow()
        {
            return Err("Approval expired or belongs to another task/session".into());
        }
        let sender = s.pending.take().ok_or("Approval already consumed")?;
        if matches!(decision, Decision::Cancel) {
            s.cancel.send_replace(true);
        }
        sender.send(decision).map_err(|_| "Approval expired".into())
    }
    pub(super) fn emit(&self, kind: &str, data: Value) {
        let mut s = self.state.lock().unwrap();
        let Some(v) = &mut s.view else {
            return;
        };
        append_event(v, kind, data);
        drop(s);
        self.changed();
    }
    pub(super) async fn approval(
        &self,
        call: &ToolCall,
        action: Value,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Decision> {
        let approval = Approval {
            id: crate::types::id(),
            call_id: call.id.clone(),
            tool: call.name.clone(),
            action,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut s = self.state.lock().unwrap();
            if *s.cancel.borrow() {
                return Err("Cancelled".into());
            }
            s.pending = Some(tx);
            let v = s.view.as_mut().unwrap();
            v.status = Status::AwaitingApproval;
            v.approval = Some(approval.clone());
        }
        self.emit("approval_required", json!(approval));
        let decision = tokio::select! {biased; _=cancel.changed()=>Err("Cancelled".into()), result=rx=>result.map_err(|_|"Approval expired".into())};
        {
            let mut s = self.state.lock().unwrap();
            s.pending = None;
            let v = s.view.as_mut().unwrap();
            v.approval = None;
            v.status = Status::Running;
        }
        self.changed();
        decision
    }
}
fn append_event(v: &mut TaskView, kind: &str, data: Value) {
    let sequence = v.events.last().map_or(1, |e| e.sequence + 1);
    let event = Event {
        id: format!("{}:{sequence}", v.id),
        sequence,
        task_id: v.id.clone(),
        kind: kind.into(),
        data,
    };
    v.audit.push(event.clone());
    v.events.push(event);
    // Bounded in-memory activity; cursors remain monotonic when older output rolls off.
    while v.events.len() > 300
        || v.events
            .iter()
            .map(|e| e.data.to_string().len())
            .sum::<usize>()
            > 2 * 1024 * 1024
    {
        v.events.remove(0);
    }
}
