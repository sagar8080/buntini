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
}
pub struct Turn {
    pub text: String,
    pub tool: Option<Value>,
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
        }
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
        Ok(Worker { child, input, output, partial, key: launch.key.clone() })
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
        let mut slot = self.worker.lock().await;
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
                tokio::select! {result=&mut startup=>{*slot=Some(result?);break;},_=tokio::time::sleep(Duration::from_millis(40))=>{if cancel.load(Ordering::Relaxed){return Err("Task cancelled.".into());}}}
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
        if worker.input.write_all(&bytes).await.is_err() {
            *slot = None;
            return Err("The local worker connection was lost. Try again.".into());
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut text = String::new();
        let mut tool = None;
        let mut failure = None;
        let result = loop {
            if cancel.load(Ordering::Relaxed) {
                break Err("Task cancelled.".into());
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
                        "pong"=>break Ok(Turn{text:msg.to_string(),tool:None}),
                        "token"=>{if let Some(t)=msg.get("text").and_then(Value::as_str){if text.len()+t.len()>64_000{break Err("The local response exceeded its safe size limit.".into());} text.push_str(t);on_token(t.into());}},
                        "tool_call"=>{if tool.is_some(){break Err("The model requested multiple tools in a single turn.".into());} tool=Some(msg);},
                        "error"=>{failure=Some(msg.get("message").and_then(Value::as_str).unwrap_or("Local model inference failed.").chars().take(2000).collect::<String>());},
                        "done"=>{if msg.get("finish_reason").and_then(Value::as_str)==Some("cancelled"){break Err("Task cancelled.".into());}if text.is_empty(){text=msg.get("text").and_then(Value::as_str).unwrap_or("").to_string();}break match failure{Some(e)=>Err(e),None=>Ok(Turn{text,tool})};},
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
        let launch = Launch { program: "python3".into(), args: vec!["-u".into(), script.to_string_lossy().into_owned()], env: vec![], key: "test".into(), start_error: "no python".into() };
        (dir, WorkerManager::new("test worker"), launch)
    }
    #[tokio::test]
    async fn partial_json_lines_survive_timer_wakeups_and_worker_is_reused() {
        let (_dir,worker,launch)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n event=json.dumps({'id':r['id'],'type':'token','text':'hello'})+'\\n'\n sys.stdout.write(event[:9]);sys.stdout.flush();time.sleep(.12);sys.stdout.write(event[9:]);sys.stdout.flush()\n print(json.dumps({'id':r['id'],'type':'done','text':'hello'}),flush=True)\n");
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
        let (_dir,worker,launch)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n if r.get('hang'): time.sleep(30)\n print(json.dumps({'id':r['id'],'type':'done','text':'recovered'}),flush=True)\n");
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
}
