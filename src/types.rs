use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Dir115 {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub id: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub parent_id: Option<Value>,
    pub pickcode: Option<Value>,
    pub sha1: Option<Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShareInfo {
    pub code: String,
    pub password: String,
    pub count: i64,
    pub total_size: u64,
    pub total_size_text: String,
    pub files: Vec<Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UploadResult {
    pub ok: bool,
    pub http_status: u16,
    pub sha1: String,
    pub size: usize,
    pub object: String,
    pub filename: String,
    pub target: String,
    pub file_id: Option<Value>,
    pub pick_code: Option<Value>,
    pub response: Value,
}
