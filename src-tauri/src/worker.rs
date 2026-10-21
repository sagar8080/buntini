use crate::types::Settings;
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
    python: String,
    model: String,
}
pub struct WorkerManager {
    worker: Mutex<Option<Worker>>,
    pub status: std::sync::Mutex<String>,
    script: PathBuf,
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
            return Err("The local worker closed unexpectedly. Check your Python environment and model path.".into());
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
    /// The MLX worker runs on this Mac, so it may see local data (see privacy.rs).
    pub fn locality(&self) -> crate::privacy::Locality {
        crate::privacy::Locality::OnDevice
    }
    pub fn new(script: PathBuf) -> Self {
        Self {
            worker: Mutex::new(None),
            status: std::sync::Mutex::new("Not started · local tools are ready".into()),
            script,
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
    async fn spawn(&self, settings: &Settings) -> Result<Worker, String> {
        if !self.script.is_file() {
            return Err("The MLX worker is missing from this installation.".into());
        }
        let mut command = Command::new(&settings.python_path);
        command
            .arg("-u")
            .arg(&self.script)
            .env("PYTHONUNBUFFERED", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child=command.spawn().map_err(|_|"Cannot start Python. Select your existing MLX virtual environment's Python executable in Settings.".to_string())?;
        let input = child.stdin.take().ok_or("Cannot open worker input.")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("Cannot open worker output.")?);
        let mut partial = Vec::new();
        let ready = tokio::time::timeout(
            Duration::from_secs(30),
            bounded_line(&mut output, &mut partial),
        )
        .await
        .map_err(|_| "The local worker did not start within 30 seconds.".to_string())??;
        let ready: Value = serde_json::from_str(&ready)
            .map_err(|_| "The local worker returned an invalid startup message.".to_string())?;
        if ready.get("type").and_then(Value::as_str) != Some("ready") {
            return Err("The local worker could not initialize. Verify MLX is installed in the selected Python environment.".into());
        }
        *self.status.lock().unwrap() = "Ready · persistent MLX worker".into();
        Ok(Worker {
            child,
            input,
            output,
            partial,
            python: settings.python_path.clone(),
            model: settings.model_path.clone(),
        })
    }
    pub async fn request<F>(
        &self,
        settings: &Settings,
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
            .map(|w| {
                w.python != settings.python_path
                    || w.model != settings.model_path
                    || w.child.try_wait().ok().flatten().is_some()
            })
            .unwrap_or(false)
        {
            if let Some(mut w) = slot.take() {
                let _ = w.child.kill().await;
            }
        }
        if slot.is_none() {
            let startup = self.spawn(settings);
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
    fn scripted_worker(source: &str) -> (tempfile::TempDir, WorkerManager, Settings) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("test_worker.py");
        std::fs::write(&script, source).unwrap();
        let worker = WorkerManager::new(script);
        let mut settings = Settings::default();
        settings.python_path = "python3".into();
        (dir, worker, settings)
    }
    #[tokio::test]
    async fn partial_json_lines_survive_timer_wakeups_and_worker_is_reused() {
        let (_dir,worker,settings)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n event=json.dumps({'id':r['id'],'type':'token','text':'hello'})+'\\n'\n sys.stdout.write(event[:9]);sys.stdout.flush();time.sleep(.12);sys.stdout.write(event[9:]);sys.stdout.flush()\n print(json.dumps({'id':r['id'],'type':'done','text':'hello'}),flush=True)\n");
        for _ in 0..2 {
            let result = worker
                .request(
                    &settings,
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
        let (_dir,worker,settings)=scripted_worker("import json,sys,time\nprint(json.dumps({'type':'ready'}),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n if r.get('hang'): time.sleep(30)\n print(json.dumps({'id':r['id'],'type':'done','text':'recovered'}),flush=True)\n");
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token.store(true, Ordering::Relaxed);
        });
        let before = Instant::now();
        assert!(worker
            .request(
                &settings,
                json!({"type":"generate","hang":true}),
                cancel,
                |_| {}
            )
            .await
            .is_err());
        assert!(before.elapsed() < Duration::from_secs(2));
        let result = worker
            .request(
                &settings,
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
