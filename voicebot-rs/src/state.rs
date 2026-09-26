//! state.rs — sessions.json 持久化 + 内存 TTS 任务队列（app.py 顶部两个全局的对应物）。

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;
use serde_json::Value;

use crate::config::{cfg, log_warn};

pub struct Sessions {
    map: Mutex<HashMap<String, Vec<Value>>>,
}

impl Sessions {
    pub fn load() -> Self {
        let mut map: HashMap<String, Vec<Value>> = HashMap::new();
        let path = cfg().base_dir.join("sessions.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<HashMap<String, Vec<Value>>>(&text) {
                Ok(m) => map = m,
                Err(e) => log_warn(&format!("载入会话失败: {e}")),
            }
        }
        Self { map: Mutex::new(map) }
    }

    pub fn history(&self, session_id: &str) -> Vec<Value> {
        self.map.lock().entry(session_id.to_string()).or_default().clone()
    }

    pub fn history_mut<F, R>(&self, session_id: &str, f: F) -> R
    where
        F: FnOnce(&mut Vec<Value>) -> R,
    {
        let mut guard = self.map.lock();
        let h = guard.entry(session_id.to_string()).or_default();
        let r = f(h);
        if h.is_empty() {
            guard.remove(session_id);
        }
        r
    }

    pub fn remove(&self, session_id: &str) {
        self.map.lock().remove(session_id);
    }

    pub fn trim(&self, session_id: &str) {
        let max = cfg().max_history;
        self.history_mut(session_id, |h| {
            if h.len() > max {
                let drain = h.len() - max;
                h.drain(..drain);
            }
        });
    }

    pub fn save(&self) {
        let guard = self.map.lock();
        let path = cfg().base_dir.join("sessions.json");
        if let Ok(text) = serde_json::to_string_pretty(&*guard) {
            if let Err(e) = std::fs::write(&path, text) {
                log_warn(&format!("保存会话失败: {e}"));
            }
        }
    }
}

pub struct TtsJob {
    pub text: String,
    pub voice: String,
    pub created: f64,
}

pub struct TtsJobs {
    jobs: Mutex<HashMap<String, TtsJob>>,
    order: Mutex<VecDeque<String>>,
}

pub const TTS_JOB_MAX: usize = 200;
pub const TTS_JOB_TTL_SECONDS: f64 = 30.0 * 60.0;

impl TtsJobs {
    pub fn new() -> Self {
        Self { jobs: Mutex::new(HashMap::new()), order: Mutex::new(VecDeque::new()) }
    }

    pub fn create(&self, text: &str, voice: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        {
            let mut order = self.order.lock();
            let mut jobs = self.jobs.lock();
            while let Some(oldest) = order.front().cloned() {
                let full = order.len() <= TTS_JOB_MAX
                    && jobs.get(&oldest).map(|j| now - j.created <= TTS_JOB_TTL_SECONDS).unwrap_or(false);
                if full {
                    break;
                }
                order.pop_front();
                jobs.remove(&oldest);
            }
        }
        let job_id = uuid::Uuid::new_v4().simple().to_string();
        self.jobs.lock().insert(job_id.clone(), TtsJob { text: text.to_string(), voice: voice.to_string(), created: now });
        self.order.lock().push_back(job_id.clone());
        format!("/api/tts/stream/{job_id}")
    }

    pub fn get(&self, job_id: &str) -> Option<(String, String)> {
        self.jobs.lock().get(job_id).map(|j| (j.text.clone(), j.voice.clone()))
    }
}
