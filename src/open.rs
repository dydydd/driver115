//! 115 开放平台(Open API)客户端。
//!
//! 与 cookie 驱动不同, 这里用 `authorization: Bearer <access_token>` 鉴权,
//! 端点在 `proapi.115.com/open/*` 与 `qrcodeapi.115.com/open/*`。
//!
//! 本模块只实现协议, 不内置任何 app_id / 密钥, 凭证全部由调用方传入。

use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

const PRO: &str = "https://proapi.115.com";
const QR: &str = "https://qrcodeapi.115.com";
const UA: &str = "Mozilla/5.0 115disk/36.2.28 115Browser/36.2.28";

/// Open API 凭证, 全部由调用方提供。
#[derive(Debug, Clone, Default)]
pub struct OpenCreds {
    pub app_id: Option<i64>,
    pub access_token: String,
    pub refresh_token: String,
    /// PKCE verifier, 缺省 64 个 '0'
    pub code_verifier: Option<String>,
}

impl OpenCreds {
    pub fn new(access_token: impl Into<String>) -> Self {
        Self { access_token: access_token.into(), ..Self::default() }
    }
}

#[derive(Clone)]
pub struct OpenClient {
    pub http: reqwest::Client,
    pub creds: OpenCreds,
}

impl OpenClient {
    pub fn new(creds: OpenCreds) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .user_agent(UA)
                .build()
                .expect("http client"),
            creds,
        }
    }

    fn bearer(&self) -> String { format!("Bearer {}", self.creds.access_token) }
    fn verifier(&self) -> String {
        self.creds.code_verifier.clone().unwrap_or_else(|| "0".repeat(64))
    }

    async fn get(&self, path: &str, q: &[(&str, String)]) -> Result<Value> {
        let mut rb = self.http.get(format!("{PRO}{path}"))
            .header("authorization", self.bearer())
            .header("Accept", "application/json");
        for (k, v) in q { rb = rb.query(&[(k, v)]); }
        let s = rb.send().await?.text().await?;
        serde_json::from_str(&s).map_err(Error::Json)
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let s = self.http.post(format!("{PRO}{path}"))
            .header("authorization", self.bearer())
            .json(body).send().await?.text().await?;
        serde_json::from_str(&s).map_err(Error::Json)
    }

    async fn post_qr(&self, path: &str, q: &[(&str, String)]) -> Result<Value> {
        let s = self.http.post(format!("{QR}{path}"))
            .header("Accept", "application/json")
            .query(q).send().await?.text().await?;
        serde_json::from_str(&s).map_err(Error::Json)
    }

    // ---- 认证 ----

    /// 获取登录二维码 (PKCE)。需要 app_id。
    pub async fn auth_device_code(&self) -> Result<Value> {
        let app = self.creds.app_id.ok_or_else(|| Error::Config("需要 app_id".into()))?;
        self.post_qr("/open/authDeviceCode", &[
            ("app_id", app.to_string()),
            ("code_challenge", self.verifier()),
            ("code_challenge_method", "md5".into()),
        ]).await
    }

    /// 扫码后用 uid 换取 access_token / refresh_token
    pub async fn device_code_to_token(&self, uid: &str) -> Result<Value> {
        self.post_qr("/open/deviceCodeToToken", &[
            ("uid", uid.to_string()), ("code_verifier", self.verifier()),
        ]).await
    }

    /// 刷新 token (旧 refresh_token 作废)
    pub async fn refresh_token(&self, refresh: &str) -> Result<Value> {
        self.post_qr("/open/refreshToken", &[("refresh_token", refresh.to_string())]).await
    }

    // ---- 用户 / 文件 ----

    pub async fn user_info(&self) -> Result<Value> { self.get("/open/user/info", &[]).await }

    pub async fn fs_files(&self, cid: &str, limit: i64, offset: i64) -> Result<Value> {
        self.get("/open/ufile/files", &[
            ("cid", cid.to_string()), ("limit", limit.to_string()),
            ("offset", offset.to_string()), ("aid", "1".into()),
        ]).await
    }

    pub async fn fs_search(&self, keyword: &str, cid: &str, limit: i64, offset: i64) -> Result<Value> {
        self.get("/open/ufile/search", &[
            ("keyword", keyword.to_string()), ("cid", cid.to_string()),
            ("limit", limit.to_string()), ("offset", offset.to_string()), ("aid", "1".into()),
        ]).await
    }

    pub async fn fs_info(&self, id: &str) -> Result<Value> {
        self.get("/open/folder/get_info", &[("id", id.to_string())]).await
    }

    pub async fn fs_move(&self, from: &str, to: &str, ids: &[String]) -> Result<Value> {
        self.post_json("/open/ufile/move", &json!({"file_ids": ids, "target_pid": to, "pid": from})).await
    }

    pub async fn fs_copy(&self, from: &str, to: &str, ids: &[String]) -> Result<Value> {
        self.post_json("/open/ufile/copy", &json!({"file_ids": ids, "target_pid": to, "pid": from})).await
    }

    pub async fn fs_delete(&self, ids: &[String]) -> Result<Value> {
        self.post_json("/open/ufile/delete", &json!({"file_ids": ids})).await
    }

    pub async fn fs_update(&self, id: &str, name: &str) -> Result<Value> {
        self.post_json("/open/ufile/update", &json!({"file_id": id, "name": name})).await
    }

    pub async fn download_url(&self, id: &str) -> Result<Value> {
        self.get("/open/ufile/downurl", &[("file_id", id.to_string())]).await
    }

    // ---- 离线下载 ----

    pub async fn offline_add_urls(&self, urls: &[String], to_cid: &str) -> Result<Value> {
        self.post_json("/open/offline/add_task_urls",
            &json!({"urls": urls.join("\n"), "wp_path_id": to_cid})).await
    }

    pub async fn offline_add_bt(&self, torrent_base64: &str, to_cid: &str) -> Result<Value> {
        self.post_json("/open/offline/add_task_bt",
            &json!({"torrent": torrent_base64, "wp_path_id": to_cid})).await
    }

    pub async fn offline_tasks(&self, page: i64) -> Result<Value> {
        self.get("/open/offline/get_task_list", &[("page", page.to_string())]).await
    }

    pub async fn offline_quota(&self) -> Result<Value> {
        self.get("/open/offline/get_quota_info", &[]).await
    }

    pub async fn offline_delete(&self, hashes: &[String]) -> Result<Value> {
        self.post_json("/open/offline/del_task", &json!({"hashes": hashes})).await
    }

    pub async fn offline_clear(&self, done: bool) -> Result<Value> {
        self.post_json("/open/offline/clear_task",
            &json!({"flag": if done { "done" } else { "all" }})).await
    }

    // ---- 回收站 ----

    pub async fn recycle_list(&self) -> Result<Value> { self.get("/open/rb/list", &[]).await }
    pub async fn recycle_delete(&self, ids: &[String]) -> Result<Value> {
        self.post_json("/open/rb/del", &json!({"file_ids": ids})).await
    }
    pub async fn recycle_revert(&self, ids: &[String]) -> Result<Value> {
        self.post_json("/open/rb/revert", &json!({"file_ids": ids})).await
    }

    // ---- 上传 ----

    pub async fn upload_init(&self, file_name: &str, sha1: &str, file_size: i64, target: &str) -> Result<Value> {
        self.post_json("/open/upload/init", &json!({
            "file_name": file_name, "fileid": sha1, "file_size": file_size,
            "target": target, "topupload": 0,
        })).await
    }

    pub async fn upload_token(&self) -> Result<Value> { self.get("/open/upload/get_token", &[]).await }

    // ---- 视频 / 会员 ----

    pub async fn video_play(&self, id: &str) -> Result<Value> {
        self.get("/open/video/play", &[("file_id", id.to_string())]).await
    }
    pub async fn video_subtitle(&self, id: &str) -> Result<Value> {
        self.get("/open/video/subtitle", &[("file_id", id.to_string())]).await
    }
    pub async fn vip_qr_url(&self) -> Result<Value> { self.get("/open/vip/qr_url", &[]).await }
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
