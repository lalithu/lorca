use lorca::{
    agent::{Limits, Status, TaskView},
    client::Client,
};
use serde_json::{Value, json};
use std::io::BufRead;
use std::{io, path::PathBuf, time::Duration};

pub fn run() -> io::Result<()> {
    tokio::runtime::Runtime::new()?
        .block_on(start())
        .map_err(io::Error::other)
}
async fn start() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let mut workspace = None;
    let mut model = "qwen3.5:9b".to_string();
    let mut host = "http://127.0.0.1:11434".to_string();
    let mut limits = Limits::default();
    let mut task = vec![];
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                workspace = Some(PathBuf::from(args.next().ok_or("Missing workspace")?))
            }
            "--model" => model = args.next().ok_or("Missing model")?,
            "--host" => host = args.next().ok_or("Missing host")?,
            "--max-iterations" => {
                limits.iterations = args
                    .next()
                    .ok_or("Missing iteration limit")?
                    .parse()
                    .map_err(|_| "Invalid iteration limit")?
            }
            "--model-timeout" => {
                limits.model_seconds = args
                    .next()
                    .ok_or("Missing model timeout")?
                    .parse()
                    .map_err(|_| "Invalid model timeout")?
            }
            "--command-timeout" => {
                limits.command_seconds = args
                    .next()
                    .ok_or("Missing command timeout")?
                    .parse()
                    .map_err(|_| "Invalid command timeout")?
            }
            "--help" | "-h" => {
                println!(
                    "lorca agent --workspace PATH [--model qwen3.5:9b] [--host URL] [--max-iterations 20] [--model-timeout 120] [--command-timeout 30] \"TASK\"\nApprovals: a = allow once, d = deny, c = cancel. Ctrl+C cancels."
                );
                return Ok(());
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown agent option: {arg}")),
            _ => task.push(arg),
        }
    }
    let workspace = workspace
        .ok_or("Use --workspace PATH")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let task = task.join(" ");
    limits.validate()?;
    if task.trim().is_empty() {
        return Err("Supply a task in quotes".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let client = Client::connect(&executable)
        .await
        .map_err(|e| e.to_string())?;
    let session = client.attach(&model, &host, false).await?;
    let result = run_task(&client, &session, &workspace, &task, limits).await;
    let _ = client
        .command(json!({"op":"close","session":session,"unload":false}))
        .await;
    result
}
async fn run_task(
    client: &Client,
    session: &str,
    workspace: &std::path::Path,
    task: &str,
    limits: Limits,
) -> Result<(), String> {
    let started=client.command(json!({"op":"agent_start","session":session,"workspace":workspace,"task":task,"limits":limits})).await?;
    let task_id = started["taskId"].as_str().ok_or("No task id")?;
    // Tokio's stdin uses a blocking pool read that can prevent Runtime shutdown.
    // A detached OS thread lets completed/cancelled CLI tasks exit immediately.
    let (input_tx, mut lines) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if input_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut sequence = 0;
    let mut pending = None::<String>;
    let mut input = true;
    let mut cancelled = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c(),if !cancelled=>{client.command(json!({"op":"agent_cancel","session":session,"taskId":task_id})).await?;cancelled=true;println!("Cancelling…");},
            line=lines.recv(),if input=>{
                match line {
                    Some(line)=>{
                        let line=line.map_err(|e|e.to_string())?;
                        let decision=match line.trim(){"a"=>"allow_once","d"=>"deny","c"=>"cancel",_=>{println!("Use a (allow once), d (deny), or c (cancel)");continue;}};
                        if decision=="cancel" {client.command(json!({"op":"agent_cancel","session":session,"taskId":task_id})).await?;cancelled=true;}
                        else if let Some(approval)=pending.take(){if let Err(e)=client.command(json!({"op":"agent_decide","session":session,"taskId":task_id,"approvalId":approval,"decision":decision})).await{println!("{}",safe(&e));}}
                        else{println!("No pending approval");}
                    },
                    None=>{input=false;client.command(json!({"op":"agent_cancel","session":session,"taskId":task_id})).await?;cancelled=true;},
                }
            },
            _=ticker.tick()=>{
                let value=client.command(json!({"op":"agent_snapshot","session":session})).await?;
                let view:TaskView=serde_json::from_value(value).map_err(|e|e.to_string())?;
                if view.id!=task_id{return Err("Task replaced".into());}
                if let Some(first)=view.events.first() && sequence>0 && first.sequence>sequence+1{println!("[Earlier activity rolled out of the bounded event buffer]");}
                for event in view.events.iter().filter(|e|e.sequence>sequence){display(&event.kind,&event.data);}
                sequence=view.events.last().map_or(sequence,|e|e.sequence);
                pending=view.approval.map(|a|a.id);
                if !view.status.active(){println!("\n{}",safe(view.result.as_deref().unwrap_or("No result")));return if view.status==Status::Failed{Err("Agent task failed".into())}else{Ok(())};}
            }
        }
    }
}
fn safe(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
fn display(kind: &str, data: &Value) {
    match kind {
        "approval_required" => {
            println!("\nAPPROVAL REQUIRED — {}", data["tool"]);
            if let Some(diff) = data["action"]["diff"].as_str() {
                println!("{}", safe(diff));
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&data["action"]).unwrap_or_default()
                );
            }
            println!("a = Allow Once · d = Deny · c = Cancel (then Enter)");
        }
        "terminal_output" => print!("{}", safe(data["output"]["text"].as_str().unwrap_or(""))),
        "tool_requested" => println!("→ {} {}", data["tool"], data["arguments"]),
        "tool_completed" => println!(
            "✓ {} {}",
            data["tool"],
            lorca::agent::bounded(&data["result"].to_string(), 400)
        ),
        "tool_failed" => println!("! {} {}", data["tool"], data["result"]),
        "model_generating" => println!("… model generating (turn {})", data["iteration"]),
        "task_started" => println!("Task started in {}", data["workspace"]),
        _ => {}
    }
}
