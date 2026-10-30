use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    partial: Vec<u8>,
    key: String,
}
/// How to start one model process that speaks the JSONL contract in worker/PROTOCOL.md.
/// A different `key` (interpreter, model folder, helper version) restarts the process.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, String)>,
    pub key: String,
    /// Shown when the program cannot be started at all.
    pub start_error: String,
}
pub struct WorkerManager {
    worker: Mutex<Option<Worker>>,
    pub status: std::sync::Mutex<String>,
    label: &'static str,
    closed: AtomicBool,
    #[cfg(test)]
    model_calls: std::sync::atomic::AtomicUsize,
}
#[derive(Debug)]
pub struct Turn {
    pub text: String,
    pub tool: Option<Value>,
    /// A cloud model's reply exactly as received (thinking, signatures), sent back on the next step.
    pub raw: Option<Value>,
}

async fn bounded_line(
    reader: &mut BufReader<ChildStdout>,
    line: &mut Vec<u8>,
) -> Result<String, String> {
    loop {
        let buf = reader
            .fill_buf()
            .await
            .map_err(|_| "Cannot read from the local worker.".to_string())?;
        if buf.is_empty() {
            return Err("The local model process closed unexpectedly. Check the model settings and try again.".into());
        }
        let take = buf
            .iter()
            .position(|b| *b == b'\n')
            .map(|x| x + 1)
            .unwrap_or(buf.len());
        if line.len() + take > 262_144 {
            return Err("The local worker exceeded its message limit.".into());
        }
        let complete = buf[take - 1] == b'\n';
        line.extend_from_slice(&buf[..take]);
        reader.consume(take);
        if complete {
            return String::from_utf8(std::mem::take(line))
                .map_err(|_| "The local worker returned invalid text.".into());
        }
    }
}
impl WorkerManager {
    pub fn new(label: &'static str) -> Self {
        Self {
            worker: Mutex::new(None),
            status: std::sync::Mutex::new("Not started · local tools are ready".into()),
            label,
            closed: AtomicBool::new(false),
            #[cfg(test)]
            model_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    /// Counts actual SDK invocations reported by the native helper, not
    /// deterministic controller bindings or a request merely reaching a worker.
    #[cfg(test)]
    pub(crate) fn model_calls(&self) -> usize {
        self.model_calls.load(Ordering::SeqCst)
    }
    pub async fn reset(&self) {
        let mut w = self.worker.lock().await;
        if let Some(mut worker) = w.take() {
            let _ = worker.child.kill().await;
        }
        *self.status.lock().unwrap() = "Not started · local tools are ready".into();
    }
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.reset().await;
    }
    async fn spawn(&self, launch: &Launch) -> Result<Worker, String> {
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .envs(launch.env.iter().map(|(key, value)| (*key, value.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| launch.start_error.clone())?;
        let input = child.stdin.take().ok_or("Cannot open worker input.")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("Cannot open worker output.")?);
        let mut partial = Vec::new();
        let ready = tokio::time::timeout(
            Duration::from_secs(30),
            bounded_line(&mut output, &mut partial),
        )
        .await
        .map_err(|_| format!("The {} did not start within 30 seconds.", self.label))??;
        let ready: Value = serde_json::from_str(&ready)
            .map_err(|_| "The local worker returned an invalid startup message.".to_string())?;
        if ready.get("type").and_then(Value::as_str) != Some("ready") {
            return Err(format!("The {} could not initialize.", self.label));
        }
        *self.status.lock().unwrap() = format!("Ready · {}", self.label);
        Ok(Worker {
            child,
            input,
            output,
            partial,
            key: launch.key.clone(),
        })
    }
    pub async fn request<F>(
        &self,
        launch: &Launch,
        mut request: Value,
        cancel: Arc<AtomicBool>,
        mut on_token: F,
    ) -> Result<Turn, String>
    where
        F: FnMut(String) + Send,
    {
        if self.closed.load(Ordering::Relaxed) {
            return Err("Buntini is closing.".into());
        }
        let mut slot = loop {
            tokio::select! {
                slot = self.worker.lock() => break slot,
                _ = tokio::time::sleep(Duration::from_millis(40)) => {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Task cancelled.".into());
                    }
                    if self.closed.load(Ordering::Relaxed) {
                        return Err("Buntini is closing.".into());
                    }
                }
            }
        };
        if self.closed.load(Ordering::Relaxed) {
            return Err("Buntini is closing.".into());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Task cancelled.".into());
        }
        if slot
            .as_mut()
            .map(|w| w.key != launch.key || w.child.try_wait().ok().flatten().is_some())
            .unwrap_or(false)
        {
            if let Some(mut w) = slot.take() {
                let _ = w.child.kill().await;
            }
        }
        if slot.is_none() {
            let startup = self.spawn(launch);
            tokio::pin!(startup);
            loop {
                tokio::select! {
                    result = &mut startup => { *slot = Some(result?); break; },
                    _ = tokio::time::sleep(Duration::from_millis(40)) => {
                        if cancel.load(Ordering::Relaxed) { return Err("Task cancelled.".into()); }
                        if self.closed.load(Ordering::Relaxed) { return Err("Buntini is closing.".into()); }
                    }
                }
            }
        }
        let worker = slot.as_mut().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        request["id"] = json!(id);
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if bytes.len() > 250_000 {
            return Err(
                "This conversation is too large for a local request. Start a shorter task.".into(),
            );
        }
        bytes.push(b'\n');
        let deadline = Instant::now() + Duration::from_secs(120);
        // A worker that stopped reading can fill its input pipe. Writing the request
        // needs the same cancellation and deadline as reading its reply.
        let written = {
            let write = worker.input.write_all(&bytes);
            tokio::pin!(write);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break Err("Task cancelled.".to_string());
                }
                if self.closed.load(Ordering::Relaxed) {
                    break Err("Buntini is closing.".to_string());
                }
                if Instant::now() >= deadline {
                    break Err("The local worker did not accept the request within two minutes. Try again.".to_string());
                }
                tokio::select! {
                    result = &mut write => break result.map_err(|_| "The local worker connection was lost. Try again.".to_string()),
                    _ = tokio::time::sleep(Duration::from_millis(40)) => {}
                }
            }
        };
        if let Err(error) = written {
            if let Some(mut worker) = slot.take() {
                let _ = worker.child.kill().await;
            }
            *self.status.lock().unwrap() = "Stopped · retry will restart worker".into();
            return Err(error);
        }
        let worker = slot.as_mut().unwrap();
        let mut text = String::new();
        let mut tool = None;
        let mut failure = None;
        let result = loop {
            if cancel.load(Ordering::Relaxed) {
                break Err("Task cancelled.".into());
            }
            if self.closed.load(Ordering::Relaxed) {
                break Err("Buntini is closing.".into());
            }
            if Instant::now() >= deadline {
                break Err("The local model timed out after two minutes. Try a shorter prompt or a smaller model.".into());
            }
            tokio::select! {
                line=bounded_line(&mut worker.output,&mut worker.partial)=> {
                    let line=match line{Ok(l)=>l,Err(e)=>break Err(e)};
                    let msg:Value=match serde_json::from_str(&line){Ok(v)=>v,Err(_)=>break Err("The local worker returned invalid JSON.".into())};
                    if msg.get("id").and_then(Value::as_str)!=Some(&id){continue;}
                    match msg.get("type").and_then(Value::as_str).unwrap_or("") {
                        "model_call"=>{
                            #[cfg(test)]
                            self.model_calls.fetch_add(1, Ordering::SeqCst);
                        },
                        "pong"=>break Ok(Turn{text:msg.to_string(),tool:None,raw:None}),
                        "token"=>{if let Some(t)=msg.get("text").and_then(Value::as_str){if text.len()+t.len()>64_000{break Err("The local response exceeded its safe size limit.".into());} text.push_str(t);on_token(t.into());}},
                        "tool_call"=>{if tool.is_some(){break Err("The model requested multiple tools in a single turn.".into());} tool=Some(msg);},
                        "error"=>{failure=Some(msg.get("message").and_then(Value::as_str).unwrap_or("Local model inference failed.").chars().take(2000).collect::<String>());},
                        "done" => {
                            let reason = msg.get("finish_reason").and_then(Value::as_str).unwrap_or("");
                            if reason == "cancelled" {
                                break Err("Task cancelled.".into());
                            }
                            if let Some(error) = failure {
                                break Err(error);
                            }
                            if reason == "error" {
                                break Err("Local model inference failed. Try again.".into());
                            }
                            if matches!(reason, "length" | "max_tokens") {
                                break Err("The local model ran out of room before finishing. Try a shorter request.".into());
                            }
                            if !matches!(reason, "stop" | "tool_call") {
                                break Err("The local worker returned an invalid completion reason. Try again.".into());
                            }
                            let Some(final_text) = msg.get("text").and_then(Value::as_str) else {
                                break Err("The local worker returned an invalid final response. Try again.".into());
                            };
                            if final_text.len() > 64_000 {
                                break Err("The local response exceeded its safe size limit.".into());
                            }
                            if !text.is_empty() && text != final_text {
                                break Err("The local worker returned an incomplete response. Try again.".into());
                            }
                            if (reason == "tool_call") != tool.is_some() {
                                break Err("The local worker returned an incomplete tool request. Try again.".into());
                            }
                            if text.is_empty() {
                                text = final_text.to_string();
                            }
                            break Ok(Turn { text, tool, raw: None });
                        },
                        _=>{}
                    }
                },
                _=tokio::time::sleep(Duration::from_millis(40))=>{}
            }
        };
        if result.is_err() {
            if let Some(mut w) = slot.take() {
                let _ = w.child.kill().await;
            }
            *self.status.lock().unwrap() = "Stopped · retry will restart worker".into();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scripted_worker(source: &str) -> (tempfile::TempDir, WorkerManager, Launch) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("test_worker.py");
        std::fs::write(&script, source).unwrap();
        let launch = Launch {
            program: "python3".into(),
            args: vec!["-u".into(), script.to_string_lossy().into_owned()],
            env: vec![],
            key: "test".into(),
            start_error: "no python".into(),
        };
        (dir, WorkerManager::new("test worker"), launch)
    }
    #[tokio::test]
    async fn partial_json_lines_survive_timer_wakeups_and_worker_is_reused() {
        let (_dir,worker,launch)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n event=json.dumps({'id':r['id'],'type':'token','text':'hello'})+'\\n'\n sys.stdout.write(event[:9]);sys.stdout.flush();time.sleep(.12);sys.stdout.write(event[9:]);sys.stdout.flush()\n print(json.dumps({'id':r['id'],'type':'done','text':'hello','finish_reason':'stop'}),flush=True)\n");
        for _ in 0..2 {
            let result = worker
                .request(
                    &launch,
                    json!({"type":"generate"}),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .await
                .unwrap();
            assert_eq!(result.text, "hello");
        }
        worker.reset().await;
    }
    #[tokio::test]
    async fn cancellation_kills_stuck_worker_and_next_request_restarts() {
        let (_dir,worker,launch)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n if r.get('hang'): time.sleep(30)\n print(json.dumps({'id':r['id'],'type':'done','text':'recovered','finish_reason':'stop'}),flush=True)\n");
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token.store(true, Ordering::Relaxed);
        });
        let before = Instant::now();
        assert!(worker
            .request(
                &launch,
                json!({"type":"generate","hang":true}),
                cancel,
                |_| {}
            )
            .await
            .is_err());
        assert!(before.elapsed() < Duration::from_secs(2));
        let result = worker
            .request(
                &launch,
                json!({"type":"generate"}),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(result.text, "recovered");
        worker.reset().await;
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_worker_that_is_not_reading_requests() {
        let (_dir, worker, launch) = scripted_worker(
            "import json,time\nprint(json.dumps({'type':'ready'}),flush=True)\ntime.sleep(30)\n",
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token.store(true, Ordering::Relaxed);
        });
        // Larger than an OS pipe buffer, but within the JSONL request allowance.
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            worker.request(
                &launch,
                json!({"type":"generate","padding":"x".repeat(200_000)}),
                cancel,
                |_| {},
            ),
        )
        .await
        .expect("cancellation must also interrupt request writes");
        assert_eq!(result.unwrap_err(), "Task cancelled.");
        assert!(worker.status.lock().unwrap().starts_with("Stopped"));
    }

    #[tokio::test]
    async fn malformed_completion_is_rejected_and_worker_can_restart() {
        let (_dir, worker, launch) = scripted_worker(concat!(
            "import json,sys\nprint(json.dumps({'type':'ready'}),flush=True)\n",
            "for line in sys.stdin:\n r=json.loads(line)\n scenario=r.get('scenario')\n",
            " if scenario=='mismatch': print(json.dumps({'id':r['id'],'type':'token','text':'partial'}),flush=True)\n",
            " if scenario=='tool': print(json.dumps({'id':r['id'],'type':'tool_call','name':'system_info','arguments':{}}),flush=True)\n",
            " text='x'*65000 if scenario=='oversized' else 'recovered'\n",
            " reason={'error':'error','truncated':'length','unknown_reason':'unknown'}.get(scenario,'stop')\n",
            " done={'id':r['id'],'type':'done','text':text,'finish_reason':reason}\n",
            " if scenario=='missing_reason': del done['finish_reason']\n",
            " if scenario=='missing_text': del done['text']\n",
            " if scenario=='nonstring_text': done['text']=['invalid']\n",
            " print(json.dumps(done),flush=True)\n",
        ));
        for scenario in [
            "mismatch",
            "tool",
            "oversized",
            "error",
            "truncated",
            "missing_reason",
            "unknown_reason",
            "missing_text",
            "nonstring_text",
        ] {
            assert!(
                worker
                    .request(
                        &launch,
                        json!({"type":"generate","scenario":scenario}),
                        Arc::new(AtomicBool::new(false)),
                        |_| {}
                    )
                    .await
                    .is_err(),
                "{scenario} must not be accepted as a complete answer"
            );
            let recovered = worker
                .request(
                    &launch,
                    json!({"type":"generate"}),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .await
                .unwrap();
            assert_eq!(recovered.text, "recovered");
        }
        worker.reset().await;
    }

    #[tokio::test]
    async fn a_queued_request_can_be_cancelled_without_waiting_for_the_active_one() {
        let (_dir, worker, launch) = scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n time.sleep(.6)\n print(json.dumps({'id':r['id'],'type':'done','text':'hello','finish_reason':'stop'}),flush=True)\n");
        let worker = Arc::new(worker);
        let active_worker = worker.clone();
        let active_launch = launch.clone();
        let active = tokio::spawn(async move {
            active_worker
                .request(
                    &active_launch,
                    json!({"type":"generate"}),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .await
        });
        // Wait until the first request owns the worker, without relying on startup speed.
        tokio::time::timeout(Duration::from_secs(2), async {
            while worker.status.lock().unwrap().starts_with("Not started") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            token.store(true, Ordering::Relaxed);
        });
        let queued = tokio::time::timeout(
            Duration::from_millis(300),
            worker.request(&launch, json!({"type":"generate"}), cancel, |_| {}),
        )
        .await
        .expect("a queued cancellation must be prompt");
        assert_eq!(queued.unwrap_err(), "Task cancelled.");
        assert_eq!(active.await.unwrap().unwrap().text, "hello");
        worker.reset().await;
    }

    #[tokio::test]
    async fn shutdown_interrupts_a_worker_that_never_finishes_starting() {
        let (_dir, worker, launch) = scripted_worker("import time\ntime.sleep(30)\n");
        let worker = Arc::new(worker);
        let active_worker = worker.clone();
        let active = tokio::spawn(async move {
            active_worker
                .request(
                    &launch,
                    json!({"type":"generate"}),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        tokio::time::timeout(Duration::from_secs(2), worker.shutdown())
            .await
            .expect("closing must interrupt startup instead of waiting 30 seconds");
        assert_eq!(active.await.unwrap().unwrap_err(), "Buntini is closing.");
    }
}
