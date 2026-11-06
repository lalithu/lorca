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
    revision_task: Option<String>,
    objective_revision: u64,
    accepting_revisions: bool,
    instructions: Vec<String>,
    remaining_iterations: Option<usize>,
}
pub struct Agent {
    pub(super) state: Mutex<State>,
    changes: watch::Sender<u64>,
    revisions: watch::Sender<u64>,
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
                revision_task: None,
                objective_revision: 1,
                accepting_revisions: true,
                instructions: Vec::new(),
                remaining_iterations: Some(0),
            }),
            changes,
            revisions: watch::channel(0).0,
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
        self.submit_continuation(request, inference, registry, vec![], None)
    }
    /// Only Core supplies records from a linked waiting task. Notes never grant authority.
    pub(crate) fn submit_continuation(
        self: &Arc<Self>,
        request: Submission,
        inference: Arc<dyn Inference>,
        registry: Registry,
        resume_evidence: Vec<Value>,
        pending_task: Option<super::context::PendingTask>,
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
        s.revision_task = None;
        s.objective_revision = 1;
        s.accepting_revisions = true;
        s.instructions = vec![task.clone()];
        s.remaining_iterations = (limits.iterations != 0).then_some(limits.iterations);
        s.view = Some(TaskView {
            thinking: Vec::new(),
            revision: 0,
            conversation_id,
            request_id,
            id: id.clone(),
            owner,
            model,
            task: task.clone(),
            contract: None,
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
            let base_history = history.clone();
            let mut history = history;
            let mut evidence = resume_evidence;
            let mut previous = pending_task;
            let mut registry = registry;
            registry.attach_inference(inference.clone());
            let result = loop {
                let result = this
                    .run(
                        workspace.as_ref(),
                        &limits,
                        inference.clone(),
                        &registry,
                        rx.clone(),
                        super::context::RunHistory {
                            messages: history.clone(),
                            evidence: evidence.clone(),
                            pending_task: previous.clone(),
                        },
                    )
                    .await;
                this.interrupt_reasoning();
                if this.revision_pending()
                    && let Err(error) = registry.settle_for_revision().await
                {
                    break Err(error);
                }
                let mut state = this.state.lock().unwrap();
                let Some(task) = state.revision_task.take() else {
                    state.accepting_revisions = false;
                    break result;
                };
                let view = state.view.as_mut().unwrap();
                previous = Some(super::context::PendingTask {
                    id: view.id.clone(),
                    objective: view.task.clone(),
                    contract: view.contract.clone(),
                    completed: false,
                });
                evidence = view.transcript.clone();
                history = base_history.clone();
                history.extend(crate::context::complete_groups(view.transcript.clone()));
                history.push(json!({"role":"user","content":task}));
                view.task = task;
                view.generating = false;
                view.status = Status::Running;
                drop(state);
                this.emit(
                    "task_revised",
                    json!({"revision":this.objective_revision(),"plansInvalidated":true}),
                );
            };
            let (mut status, mut kind, mut result) = if *rx.borrow() {
                (
                    Status::Cancelled,
                    "task_cancelled",
                    "Task cancelled; no pending approvals will execute".into(),
                )
            } else {
                match result {
                    Ok(text) => (Status::Completed, "task_completed", text),
                    Err(e) if e.starts_with("Waiting for input: ") => (
                        Status::WaitingForInput,
                        "task_waiting_for_input",
                        e[19..].into(),
                    ),
                    Err(e) if e.starts_with("Task incomplete:") => {
                        (Status::Incomplete, "task_incomplete", e)
                    }
                    Err(e) if e.starts_with("Task denied:") => (Status::Denied, "task_denied", e),
                    Err(e) => (Status::Failed, "task_failed", e),
                }
            };
            let mut s = this.state.lock().unwrap();
            if *s.cancel.borrow() {
                status = Status::Cancelled;
                kind = "task_cancelled";
                result = "Task cancelled; completed effects retained, no further actions executed"
                    .into();
            }
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
    pub(crate) fn reasoning_update(
        &self,
        request_id: String,
        stage: ThinkingStage,
        state: String,
        text: String,
    ) {
        let mut s = self.state.lock().unwrap();
        let Some(view) = s.view.as_mut() else {
            return;
        };
        if state == "started" || state == "off" || state == "unavailable" {
            if view.thinking.len() >= 64 {
                view.thinking.remove(0);
            }
            view.thinking.push(ThinkingBlock {
                id: request_id.clone(),
                stage,
                state: if state == "started" {
                    "running".into()
                } else {
                    state.clone()
                },
                text: String::new(),
                after_sequence: view.audit.last().map_or(0, |e| e.sequence),
                text_offset: view.assistant_text.len(),
                truncated: false,
            });
        }
        let used: usize = view.thinking.iter().map(|v| v.text.len()).sum();
        if let Some(block) = view.thinking.iter_mut().find(|b| b.id == request_id) {
            if state == "delta" {
                let mut end = text.len().min(65536_usize.saturating_sub(used));
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                block.text.push_str(&text[..end]);
                block.truncated |= end < text.len();
            } else if state != "started" {
                block.state = state;
            }
        }
        drop(s);
        self.changed();
    }
    fn interrupt_reasoning(&self) {
        let mut s = self.state.lock().unwrap();
        if let Some(v) = s.view.as_mut() {
            for b in &mut v.thinking {
                if b.state == "running" {
                    b.state = "interrupted".into();
                }
            }
        }
    }
    pub(super) fn begin_action(&self, revision: u64, data: Value) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        if s.revision_task.is_some() || s.objective_revision != revision || *s.cancel.borrow() {
            return Err("Task revised or cancelled before execution".into());
        }
        append_event(
            s.view.as_mut().ok_or("Task disappeared")?,
            "tool_started",
            data,
        );
        drop(s);
        self.changed();
        Ok(())
    }
    pub(super) fn take_iteration(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        match &mut s.remaining_iterations {
            Some(0) => false,
            Some(remaining) => {
                *remaining -= 1;
                true
            }
            None => true,
        }
    }
    pub(super) fn instruction_history(&self) -> Vec<String> {
        self.state.lock().unwrap().instructions.clone()
    }
    pub fn objective_revision(&self) -> u64 {
        self.state.lock().unwrap().objective_revision
    }
    pub(super) fn revision_pending(&self) -> bool {
        self.state.lock().unwrap().revision_task.is_some()
    }
    pub(super) fn revision_receiver(&self) -> watch::Receiver<u64> {
        self.revisions.subscribe()
    }
    /// Queue only user-authored corrections. Actions already started settle before reuse.
    pub fn revise(&self, owner: &str, task_id: &str, text: &str) -> Result<()> {
        if text.trim().is_empty() || text.len() > 8000 {
            return Err("Correction must contain 1–8000 bytes".into());
        }
        let mut s = self.state.lock().unwrap();
        let v = s.view.as_ref().ok_or("No task")?;
        if v.owner != owner || v.id != task_id || !v.status.active() {
            return Err("No active task belongs to this session".into());
        }
        if !s.accepting_revisions {
            return Err("Task is settling; submit the correction after completion".into());
        }
        if s.objective_revision >= 8 || s.revision_task.is_some() {
            return Err("A correction is pending or the revision limit was reached".into());
        }
        s.objective_revision += 1;
        s.revision_task = Some(text.into());
        s.instructions.push(text.into());
        s.pending = None; // Closing the approval channel invalidates even an outstanding prepared action.
        s.view.as_mut().unwrap().approval = None;
        self.revisions.send_modify(|n| *n += 1);
        drop(s);
        self.emit(
            "task_revision_requested",
            json!({"instruction":text,"revision":self.objective_revision()}),
        );
        Ok(())
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
// Presentation metadata only: offsets refer to the existing streamed UTF-8 text.
// It rides the existing audit/event persistence, including bounded-buffer snapshots.
fn append_event(v: &mut TaskView, kind: &str, mut data: Value) {
    let now = presentation_time_ns();
    let started = v
        .audit
        .first()
        .and_then(|e| e.data["presentation"]["startedNanos"].as_u64())
        .unwrap_or(now);
    data["presentation"] = json!({
        "textBytes": v.assistant_text.len(),
        "startedNanos": started,
        "elapsedMillis": now.saturating_sub(started) / 1_000_000,
        "clock": if cfg!(target_os = "macos") { "mac_continuous" } else { "unix" }
    });
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

// macOS's continuous clock advances during sleep and is shared across processes,
// so a reconnect can restore elapsed time without trusting a wall-clock estimate.
#[cfg(target_os = "macos")]
fn presentation_time_ns() -> u64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    let mut scale = Timebase { numer: 0, denom: 0 };
    // SAFETY: the system fills this valid timebase struct; neither call owns memory.
    unsafe {
        mach_timebase_info(&mut scale);
        (u128::from(mach_continuous_time()) * u128::from(scale.numer)
            / u128::from(scale.denom.max(1))) as u64
    }
}
#[cfg(not(target_os = "macos"))]
fn presentation_time_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
