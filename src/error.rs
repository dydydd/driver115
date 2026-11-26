pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Http(reqwest::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    Api(String),
    Config(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => write!(f, "HTTP 请求失败: {e}"),
            Error::Json(e) => write!(f, "JSON 解析失败: {e}"),
            Error::Io(e) => write!(f, "IO 错误: {e}"),
            Error::Yaml(e) => write!(f, "YAML 解析失败: {e}"),
            Error::Api(s) => write!(f, "115 API 错误: {s}"),
            Error::Config(s) => write!(f, "配置错误: {s}"),
        }
    }
}
impl std::error::Error for Error {}

impl From<reqwest::Error> for Error { fn from(e: reqwest::Error) -> Self { Error::Http(e) } }
impl From<std::io::Error> for Error { fn from(e: std::io::Error) -> Self { Error::Io(e) } }
impl From<serde_json::Error> for Error { fn from(e: serde_json::Error) -> Self { Error::Json(e) } }
impl From<serde_yaml::Error> for Error { fn from(e: serde_yaml::Error) -> Self { Error::Yaml(e) } }
