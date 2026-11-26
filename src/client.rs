//! 轻量客户端: 提供 115 驱动所需的 HTTP 客户端与配置访问。

use crate::error::Result;
use serde_yaml::Value;
use std::sync::{Arc, RwLock};

/// 115 客户端。持有 HTTP 客户端与可写的配置(与 115driver 的 config.yaml 同结构)。
#[derive(Clone)]
pub struct Client {
    pub http: reqwest::Client,
    pub cfg: Arc<Config>,
}

impl Client {
    pub fn new(cookie: &str) -> Self {
        let mut cfg = Config::default();
        cfg.set_cookie(cookie);
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 115Browser/27.0.2.8")
                .build()
                .expect("failed to build HTTP client"),
            cfg: Arc::new(cfg),
        }
    }

    /// 从 115driver 风格的 config.yaml 加载
    pub fn from_yaml(path: &str) -> Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let v: Value = serde_yaml::from_str(&s)?;
        Ok(Self {
            http: Self::new("").http,
            cfg: Arc::new(Config::from_yaml(&v)),
        })
    }

    /// 从配置写回文件(可选)
    pub fn save_yaml(&self, path: &str) -> Result<()> {
        let g = self.cfg.data.read().unwrap();
        let s = serde_yaml::to_string(&*g)?;
        std::fs::write(path, s)?;
        Ok(())
    }
}

/// 最小配置存储, 支持 `get("settings_115")` 与 `update()`(内部可变)。
#[derive(Debug, Default)]
pub struct Config {
    data: RwLock<Value>,
}

impl Config {
    fn set_cookie(&mut self, cookie: &str) {
        let mut m = serde_yaml::Mapping::new();
        m.insert(Value::String("cookies".into()), Value::String(cookie.to_string()));
        m.insert(Value::String("status".into()), Value::Bool(true));
        let arr = Value::Sequence(vec![Value::Mapping(m)]);
        let mut root = self.data.write().unwrap();
        if let Value::Mapping(rm) = &mut *root {
            rm.insert(Value::String("settings_115".into()), arr);
        } else {
            let mut rm = serde_yaml::Mapping::new();
            rm.insert(Value::String("settings_115".into()), arr);
            *root = Value::Mapping(rm);
        }
    }

    fn from_yaml(v: &Value) -> Self {
        Self { data: RwLock::new(v.clone()) }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.read().unwrap().get(key).cloned()
    }

    pub fn update<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Value),
    {
        let mut g = self.data.write().unwrap();
        f(&mut *g);
        Ok(())
    }
}
