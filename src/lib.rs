//! `115driver-115` — 115.com 网盘的纯 Rust 客户端。
//!
//! 从 115driver 的 Rust 重写版中拆出, 保留全部真实 API 交互与 115 私有密码学实现。
//!
//! 能力:
//! - cookie 登录态 / 列目录 / 网盘内搜索
//! - 分享解析 / 快照 / 转存
//! - 离线下载添加 / 任务列表 / 删除 / 文件夹列表
//! - 批量移动 / 删除
//! - 扫码登录(申请二维码 + 轮询 + SSO 换 cookie)
//! - **秒传/普通上传**: sig/token/k_ec 签名 + AES-128-CBC + CRC32 + LZ4 + OSS 回调
//!
//! 所有请求使用 cookie 鉴权, 不需要 OAuth App。

pub mod client;
pub mod compat;
pub mod driver;
pub mod open;
pub mod error;
pub mod types;

pub use client::Client;
pub use open::{OpenClient, OpenCreds};
pub use error::{Error, Result};
