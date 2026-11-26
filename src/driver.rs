//! 115 网盘直连客户端(只用 cookie, 不依赖 Python 的 p115client)。
//!
//! 列目录接口历史上换过域名, 这里 proapi 优先、webapi 兜底,
//! 失败时把 state/errno/错误信息原样带回, 方便定位是 cookie 过期还是网络问题。

use serde_json::{json, Value};
use tracing::warn;

use crate::client::Client;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 115Browser/27.0.2.8";

pub struct Dir115 {
    pub id: String,
    pub name: String,
}

/// 取 115 账号 cookie。优先 `settings_115[0]`, 支持 cookie/cookies 两种字段名。
pub fn cookie(st: &Client) -> Result<String, String> {
    let seq = st
        .cfg
        .get("settings_115")
        .and_then(|v| v.as_sequence().cloned())
        .unwrap_or_default();
    // 统一成 owned 值, 避免 &yaml / &json 混用
    let first: serde_yaml::Value = match seq.first() {
        Some(v) => v.clone(),
        None => match st.cfg.get("settings_115") {
            Some(v) if v.is_mapping() => v,
            _ => return Err("未配置 115 账号".into()),
        },
    };
    if let Some(status) = first.get("status") {
        if status.as_bool() == Some(false) {
            return Err("115 账号已停用(status=false)".into());
        }
    }
    let c = first
        .get("cookies")
        .or_else(|| first.get("cookie"))
        .or_else(|| first.get("cookie_str"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if c.is_empty() {
        return Err("115 cookie 为空".into());
    }
    Ok(c)
}

fn parse_list(v: &Value) -> Result<Vec<Dir115>, String> {
    let state = v.get("state");
    if state.and_then(|x| x.as_bool()) == Some(false) {
        let errno = v
            .get("errno")
            .or_else(|| v.get("error"))
            .map(|x| x.to_string())
            .unwrap_or_default();
        let msg = v
            .get("error")
            .or_else(|| v.get("message"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        return Err(format!("115 返回失败 state=false errno={errno} msg={msg}"));
    }
    let arr = v
        .get("data")
        .and_then(|d| {
            if d.is_array() {
                Some(d.clone())
            } else {
                d.get("data").or_else(|| d.get("list")).cloned()
            }
        })
        .unwrap_or_else(|| json!([]));
    let mut out = Vec::new();
    if let Some(list) = arr.as_array() {
        for it in list {
            // 只取目录: 有 cid 且没有 fid
            let cid = it.get("cid").or_else(|| it.get("id")).and_then(|x| {
                if x.is_string() {
                    x.as_str().map(|s| s.to_string())
                } else {
                    x.as_i64().map(|n| n.to_string())
                }
            });
            let Some(cid) = cid else { continue };
            let is_dir = it
                .get("is_dir")
                .and_then(|x| x.as_bool())
                .or_else(|| it.get("cid").map(|c| !c.is_null()))
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let name = it
                .get("n")
                .or_else(|| it.get("name"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            out.push(Dir115 { id: cid, name });
        }
    }
    Ok(out)
}

/// 列某个目录的直接子目录。`cid="0"` 是根。
pub async fn list_dirs(st: &Client, cid: &str) -> Result<Vec<Dir115>, String> {
    let ck = cookie(st)?;
    let mut last_err = String::from("no attempt");
    for base in ["https://proapi.115.com", "https://webapi.115.com"] {
        let url = format!("{base}/files");
        let resp = st
            .http
            .get(&url)
            .header("Cookie", &ck)
            .header("User-Agent", UA)
            .header("Referer", "https://115.com/")
            .header("Accept", "application/json, text/plain, */*")
            .query(&[
                ("aid", "1"),
                ("cid", cid),
                ("o", "user_ptime"),
                ("asc", "0"),
                ("offset", "0"),
                ("show_dir", "1"),
                ("limit", "1000"),
                ("snap", "0"),
                ("natsort", "1"),
                ("format", "json"),
                ("cur", "0"),
            ])
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                if !status.is_success() {
                    last_err = format!("{base} HTTP {status}: {}", &body[..body.len().min(120)]);
                    continue;
                }
                let v: Value = match serde_json::from_str(&body) {
                    Ok(v) => v,
                    Err(e) => {
                        last_err = format!("{base} 返回非 JSON({e}): {}", &body[..body.len().min(120)]);
                        continue;
                    }
                };
                match parse_list(&v) {
                    Ok(list) => return Ok(list),
                    Err(e) => {
                        last_err = format!("{base}: {e}");
                        continue;
                    }
                }
            }
            Err(e) => {
                warn!("115 请求 {base} 失败: {e}");
                last_err = format!("{base}: {e}");
            }
        }
    }
    Err(last_err)
}

/// 登录态自检: 能列出根目录即视为有效
pub async fn check(st: &Client) -> Value {
    match list_dirs(st, "0").await {
        Ok(list) => json!({"ok": true, "root_dirs": list.len(),
            "sample": list.iter().take(5).map(|d| json!({"id": d.id, "name": d.name})).collect::<Vec<_>>()}),
        Err(e) => json!({"ok": false, "message": e}),
    }
}

// ===================== 分享: 解析 / 快照 / 转存 =====================

/// 从分享链接里解析 (share_code, receive_code)
/// 支持 https://115.com/s/xxxx?password=yyyy 、 https://115cdn.com/s/xxxx 、
/// 以及直接给 share_code 的形式
pub fn parse_share(url: &str) -> Result<(String, String), String> {
    let raw = url.trim();
    if raw.is_empty() {
        return Err("分享链接为空".into());
    }
    // 纯 code
    if !raw.contains('/') && !raw.contains('?') {
        return Ok((raw.to_string(), String::new()));
    }
    let mut code = String::new();
    for marker in ["/s/", "share_code=", "/share/"] {
        if let Some(pos) = raw.find(marker) {
            let rest = &raw[pos + marker.len()..];
            let end = rest
                .find(|c: char| c == '?' || c == '&' || c == '/' || c == '#')
                .unwrap_or(rest.len());
            code = rest[..end].to_string();
            if !code.is_empty() {
                break;
            }
        }
    }
    if code.is_empty() {
        return Err(format!("无法从链接解析 share_code: {raw}"));
    }
    let mut pw = String::new();
    for marker in ["password=", "pwd=", "receive_code=", "code="] {
        if let Some(pos) = raw.find(marker) {
            let rest = &raw[pos + marker.len()..];
            let end = rest.find(|c: char| c == '&' || c == '#').unwrap_or(rest.len());
            pw = rest[..end].to_string();
            if !pw.is_empty() {
                break;
            }
        }
    }
    Ok((code, pw))
}

fn space_str(n: i64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.2} {}", U[i])
    }
}

/// 分享快照: 返回 {count, total_size, files:[...]}
pub async fn share_snap(
    st: &Client,
    share_code: &str,
    receive_code: &str,
) -> Result<Value, String> {
    let ck = cookie(st)?;
    // webapi 是权威端点(proapi 没有 /share/snap, 会 404 盖掉真实原因)
    let mut last = String::from("no attempt");
    for base in ["https://webapi.115.com"] {
        let url = format!("{base}/share/snap");
        let r = st
            .http
            .get(&url)
            .header("Cookie", &ck)
            .header("User-Agent", UA)
            .header("Referer", "https://115.com/")
            .query(&[
                ("share_code", share_code),
                ("receive_code", receive_code),
                ("offset", "0"),
                ("limit", "100"),
                ("asc", "1"),
            ])
            .send()
            .await;
        match r {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    last = format!("{base} HTTP {status}");
                    continue;
                }
                let v: Value = match serde_json::from_str(&body) {
                    Ok(v) => v,
                    Err(e) => {
                        last = format!("{base} 非JSON({e})");
                        continue;
                    }
                };
                if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
                    let em = v
                        .get("error_msg")
                        .or_else(|| v.get("error"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    // 需要提取码
                    if em.contains("提取码") || em.contains("password") {
                        return Err(format!("需要提取码或提取码错误: {em}"));
                    }
                    if em.contains("参数错误") {
                        return Err(format!(
                            "分享码或提取码不正确(115: {em}) share_code={share_code} receive_code={receive_code:?}"
                        ));
                    }
                    return Err(format!("115: {em}"));
                }
                let d = v.get("data").cloned().unwrap_or_else(|| json!({}));
                let list = d.get("list").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                let total: i64 = list
                    .iter()
                    .map(|x| x.get("s").and_then(|s| s.as_i64()).unwrap_or(0))
                    .sum();
                let files: Vec<Value> = list
                    .iter()
                    .take(200)
                    .map(|x| {
                        json!({
                            "id": x.get("fid").or_else(|| x.get("cid")).cloned().unwrap_or(Value::Null),
                            "name": x.get("n").cloned().unwrap_or(json!("")),
                            "size": x.get("s").and_then(|s| s.as_i64()).unwrap_or(0),
                            "is_dir": x.get("cid").map(|c| !c.is_null()).unwrap_or(false),
                        })
                    })
                    .collect();
                return Ok(json!({
                    "share_code": share_code,
                    "receive_code": receive_code,
                    "count": list.len(),
                    "total_size": total,
                    "total_size_text": space_str(total),
                    "files": files,
                }));
            }
            Err(e) => last = format!("{base}: {e}"),
        }
    }
    Err(last)
}

/// 转存分享到指定目录
pub async fn share_receive(
    st: &Client,
    share_code: &str,
    receive_code: &str,
    file_ids: &str,
    to_cid: &str,
) -> Result<Value, String> {
    let ck = cookie(st)?;
    let url = "https://webapi.115.com/share/receive";
    let r = st
        .http
        .post(url)
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .form(&[
            ("share_code", share_code),
            ("receive_code", receive_code),
            ("file_id", file_ids),
            ("cid", to_cid),
        ])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", &body[..body.len().min(160)]));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(160)]))?;
    if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
        let em = v
            .get("error_msg")
            .or_else(|| v.get("error"))
            .and_then(|x| x.as_str())
            .unwrap_or("未知错误");
        return Err(format!("转存失败: {em}"));
    }
    Ok(json!({"share_code": share_code, "to_cid": to_cid,
        "raw": v.get("data").cloned().unwrap_or(Value::Null)}))
}

/// 离线下载(磁力/ED2K/HTTP)。
///
/// 实测: `webapi.115.com/offine/task` 已废弃(返回"服务器开小差了"),
/// `webapi.115.com/offline/task` 直接 404,
/// 真正可用的是老练接口 `115.com/web/lixian/?ct=lixian&ac=add_task_url`。
pub async fn offline_add(st: &Client, url: &str, to_cid: &str) -> Result<Value, String> {
    let ck = cookie(st)?;
    let u = url.trim();
    if u.is_empty() {
        return Err("缺少 url".into());
    }
    let cid = if to_cid.trim().is_empty() { "0" } else { to_cid };
    let full = "https://115.com/web/lixian/?ct=lixian&ac=add_task_url";
    let r = st
        .http
        .post(full)
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .header("Accept", "application/json, text/plain, */*")
        .form(&[("url", u), ("wp_path_id", cid)])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", &body[..body.len().min(160)]));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(160)]))?;
    if v.get("state").and_then(|x| x.as_bool()) != Some(true) {
        let em = v
            .get("error")
            .or_else(|| v.get("error_msg"))
            .or_else(|| v.get("message"))
            .and_then(|x| x.as_str())
            .unwrap_or("未知错误");
        return Err(format!("提交离线任务失败: {em}"));
    }
    Ok(json!({
        "url": u,
        "to_cid": cid,
        "info_hash": v.get("info_hash").cloned().unwrap_or(Value::Null),
        "name": v.get("name").cloned().unwrap_or(Value::Null),
    }))
}

/// 离线任务列表
pub async fn offline_tasks(st: &Client) -> Result<Vec<Value>, String> {
    let ck = cookie(st)?;
    let r = st
        .http
        .get("https://115.com/web/lixian/?ct=lixian&ac=task_lists")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    let arr = v.get("tasks").and_then(|t| t.as_array()).cloned().unwrap_or_default();
    let out = arr
        .iter()
        .map(|t| {
            let pct = t.get("percentDone").and_then(|x| x.as_f64()).unwrap_or(0.0);
            json!({
                "info_hash": t.get("info_hash").cloned().unwrap_or(Value::Null),
                "name": t.get("name").cloned().unwrap_or(json!("")),
                "size": t.get("size").cloned().unwrap_or(json!(0)),
                "status": t.get("status").cloned().unwrap_or(Value::Null),
                "percent": pct,
                "done": pct >= 100.0,
                "add_time": t.get("add_time").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(out)
}

/// 删除离线任务(按 info_hash, 逗号分隔可删多个)
pub async fn offline_delete(st: &Client, hashes: &str) -> Result<Value, String> {
    let ck = cookie(st)?;
    let h = hashes.trim();
    if h.is_empty() {
        return Err("缺少 info_hash".into());
    }
    let r = st
        .http
        .post("https://115.com/web/lixian/?ct=lixian&ac=task_del")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .form(&[("hash[0]", h), ("flag", "0")])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
        let em = v.get("error").and_then(|x| x.as_str()).unwrap_or("未知错误");
        return Err(format!("删除失败: {em}"));
    }
    Ok(json!({"deleted": h, "raw": v}))
}

/// 移动文件/文件夹: 从 pid 移到 target_cid
pub async fn move_files(
    st: &Client,
    file_ids: &str,
    from_pid: &str,
    to_cid: &str,
) -> Result<Value, String> {
    let ck = cookie(st)?;
    let ids: Vec<&str> = file_ids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        return Err("缺少 fid".into());
    }
    // 115 的批量参数是 fid[0]=..&fid[1]=..
    let mut form: Vec<(String, String)> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        form.push((format!("fid[{i}]"), (*id).to_string()));
    }
    form.push(("pid".into(), if from_pid.trim().is_empty() { "0".into() } else { from_pid.into() }));
    form.push(("cid".into(), if to_cid.trim().is_empty() { "0".into() } else { to_cid.into() }));
    let r = st
        .http
        .post("https://webapi.115.com/files/move")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
        let em = v
            .get("error")
            .or_else(|| v.get("error_msg"))
            .and_then(|x| x.as_str())
            .unwrap_or("未知错误");
        return Err(format!("移动失败: {em}"));
    }
    Ok(json!({"moved": ids.len(), "from_pid": from_pid, "to_cid": to_cid}))
}

/// 批量删除(进回收站)
pub async fn delete_files(st: &Client, file_ids: &str) -> Result<usize, String> {
    let ck = cookie(st)?;
    let ids: Vec<&str> = file_ids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        return Err("缺少 fid".into());
    }
    let mut form: Vec<(String, String)> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        form.push((format!("fid[{i}]"), (*id).to_string()));
    }
    let r = st
        .http
        .post("https://webapi.115.com/rb/delete")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
        let em = v.get("error").and_then(|x| x.as_str()).unwrap_or("未知错误");
        return Err(format!("删除失败: {em}"));
    }
    Ok(ids.len())
}

/// 网盘内文件搜索
pub async fn search(
    st: &Client,
    keyword: &str,
    cid: &str,
    limit: usize,
    offset: usize,
) -> Result<(i64, Vec<Value>), String> {
    let ck = cookie(st)?;
    let kw = keyword.trim();
    if kw.is_empty() {
        return Err("缺少搜索关键词".into());
    }
    let cid = if cid.trim().is_empty() { "0" } else { cid.trim() };
    let r = st
        .http
        .get("https://webapi.115.com/files/search")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .header("Referer", "https://115.com/")
        .query(&[
            ("search_value", kw),
            ("cid", cid),
            ("limit", &limit.to_string()),
            ("offset", &offset.to_string()),
            ("format", "json"),
            ("aid", "1"),
            ("show_dir", "1"),
        ])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", &body[..body.len().min(140)]));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    if v.get("state").and_then(|x| x.as_bool()) == Some(false) {
        let em = v.get("error").and_then(|x| x.as_str()).unwrap_or("未知错误");
        return Err(format!("搜索失败: {em}"));
    }
    let count = v.get("count").and_then(|x| x.as_i64()).unwrap_or(0);
    let arr = v.get("data").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let items = arr
        .iter()
        .map(|it| {
            // 115 搜索结果: 目录只有 cid; 文件有 fid, cid 是它所在的父目录
            let has_fid = it.get("fid").map(|x| !x.is_null()).unwrap_or(false);
            let is_dir = !has_fid && it.get("cid").map(|x| !x.is_null()).unwrap_or(false);
            let id = it
                .get("fid")
                .filter(|x| !x.is_null())
                .or_else(|| it.get("cid"))
                .and_then(|x| {
                    if x.is_string() { x.as_str().map(|s| s.to_string()) } else { x.as_i64().map(|n| n.to_string()) }
                })
                .unwrap_or_default();
            json!({
                "id": id,
                "name": it.get("n").cloned().unwrap_or(json!("")),
                "is_dir": is_dir,
                "size": it
                    .get("s")
                    .map(|x| {
                        if let Some(n) = x.as_i64() {
                            n
                        } else if let Some(s) = x.as_str() {
                            s.parse::<i64>().unwrap_or(0)
                        } else {
                            0
                        }
                    })
                    .unwrap_or(0),
                "parent_id": it.get("pid").cloned().unwrap_or(Value::Null),
                "pickcode": it.get("pc").cloned().unwrap_or(Value::Null),
                "sha1": it.get("sha").cloned().unwrap_or(Value::Null),
                "mtime": it.get("t").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok((count, items))
}

// ===================== 扫码登录(115 网页版) =====================

const QR_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 1) 申请二维码: 返回 uid / sign / 二维码 PNG(base64) / 扫码跳转链接
pub async fn qr_start(st: &Client) -> Result<Value, String> {
    let (uid, sign, qrcode_url) = fetch_qr_token(st).await?;
    // 2) 取二维码图片
    let png = st
        .http
        .get("https://qrcodeapi.115.com/api/1.0/web/1.0/qrcode")
        .header("User-Agent", QR_UA)
        .header("Referer", "https://115.com/")
        .query(&[("uid", uid.as_str())])
        .send()
        .await
        .map_err(|e| format!("取二维码失败: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("读二维码失败: {e}"))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(json!({
        "uid": uid,
        "sign": sign,
        "qrcode_url": qrcode_url,
        "image_base64": b64,
        "image_mime": "image/png",
        "image_bytes": png.len(),
        "hint": "用 115 App 扫码后调用 /qrlogin/poll 轮询",
    }))
}

async fn fetch_qr_token(st: &Client) -> Result<(String, String, String), String> {
    let r = st
        .http
        .get("https://qrcodeapi.115.com/api/1.0/web/1.0/token")
        .header("User-Agent", QR_UA)
        .header("Referer", "https://115.com/")
        .send()
        .await
        .map_err(|e| format!("申请二维码失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    let d = v.get("data").cloned().unwrap_or_else(|| json!({}));
    let uid = d.get("uid").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let sign = d.get("sign").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let qurl = d.get("qrcode").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if uid.is_empty() {
        return Err(format!("未取到 uid: {}", &body[..body.len().min(140)]));
    }
    Ok((uid, sign, qurl))
}

/// 3) 轮询扫码状态。确认后自动换取 cookie 并写回配置。
pub async fn qr_poll(st: &Client, uid: &str, sign: &str) -> Result<Value, String> {
    if uid.trim().is_empty() {
        return Err("缺少 uid".into());
    }
    let ts = chrono::Utc::now().timestamp();
    let r = st
        .http
        .get("https://qrcodeapi.115.com/api/1.0/web/1.0/status")
        .header("User-Agent", QR_UA)
        .header("Referer", "https://115.com/")
        .query(&[
            ("uid", uid),
            ("time", &ts.to_string()),
            ("sign", sign),
        ])
        .send()
        .await
        .map_err(|e| format!("轮询失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("返回非 JSON({e}): {}", &body[..body.len().min(140)]))?;
    let state = v.get("state").and_then(|x| x.as_bool());
    let code = v.get("code").and_then(|x| x.as_i64()).unwrap_or(0);
    let msg = v.get("message").and_then(|x| x.as_str()).unwrap_or("");

    // 未确认: state=false 且 code 90038(请扫描) / 其他等待码
    let confirmed = state == Some(true) || (state.is_none() && code == 0 && v.get("data").is_some());
    if !confirmed {
        let phase = if code == 90038 { "waiting_scan" } else { "pending" };
        return Ok(json!({"phase": phase, "code": code, "message": msg,
            "confirmed": false, "raw": v}));
    }

    // 4) 已确认 -> 换 cookie
    let cookie = fetch_sso_cookie(st).await.unwrap_or_default();
    let saved = if cookie.is_empty() {
        false
    } else {
        save_cookie_to_config(st, &cookie)
    };
    Ok(json!({
        "phase": "confirmed",
        "confirmed": true,
        "code": code,
        "cookie_len": cookie.len(),
        "cookie_saved": saved,
        "message": if saved { "登录成功, cookie 已写入配置" } else { "已确认, 但换取 cookie 失败或未保存" },
        "raw": v,
    }))
}

/// 换取 SSO cookie
async fn fetch_sso_cookie(st: &Client) -> Result<String, String> {
    let r = st
        .http
        .get("https://passportapi.115.com/app/1.0/web/1.0/check/sso")
        .header("User-Agent", QR_UA)
        .header("Referer", "https://115.com/")
        .send()
        .await
        .map_err(|e| format!("SSO 请求失败: {e}"))?;
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in r.headers().iter() {
        if k.as_str().eq_ignore_ascii_case("set-cookie") {
            if let Ok(s) = v.to_str() {
                let kv = s.split(';').next().unwrap_or("").trim();
                if kv.contains('=') {
                    parts.push(kv.to_string());
                }
            }
        }
    }
    if parts.is_empty() {
        return Err("SSO 未返回 cookie".into());
    }
    Ok(parts.join("; "))
}

/// 把新 cookie 写回 settings_115[0].cookies
fn save_cookie_to_config(st: &Client, cookie: &str) -> bool {
    let mut seq = st
        .cfg
        .get("settings_115")
        .and_then(|v| v.as_sequence().cloned())
        .unwrap_or_default();
    if seq.is_empty() {
        let mut m = serde_yaml::Mapping::new();
        m.insert(serde_yaml::Value::String("name".into()), serde_yaml::Value::String("115".into()));
        m.insert(serde_yaml::Value::String("id".into()),
                 serde_yaml::Value::String(format!("qr{}", chrono::Utc::now().timestamp())));
        m.insert(serde_yaml::Value::String("status".into()), serde_yaml::Value::Bool(true));
        seq.push(serde_yaml::Value::Mapping(m));
    }
    let first = &mut seq[0];
    if let serde_yaml::Value::Mapping(m) = first {
        m.insert(serde_yaml::Value::String("cookies".into()),
                 serde_yaml::Value::String(cookie.to_string()));
        m.insert(serde_yaml::Value::String("status".into()), serde_yaml::Value::Bool(true));
    }
    let ok = st
        .cfg
        .update(|cfg| {
            if let serde_yaml::Value::Mapping(m) = cfg {
                m.insert(serde_yaml::Value::String("settings_115".into()),
                         serde_yaml::Value::Sequence(seq.clone()));
            }
        })
        .is_ok();
    ok
}

// ===================== 上传初始化 / 秒传(sig + token + AES + LZ4) =====================
//
// 移植自 p115cipher: 所谓 ECDH 实际是**固定 key/iv 的 AES-128-CBC**,
// token 是 crc32 校验的定长结构; 响应要 AES 解密后再按 LZ4 块解压。

const AES_KEY: [u8; 16] = [
    0xfb, 0x1a, 0x19, 0xd6, 0x52, 0xf5, 0xaa, 0xf7, 0xbc, 0x65, 0x1d, 0x0f, 0x69, 0xbf, 0x42, 0x2f,
];
const AES_IV: [u8; 16] = [
    0x69, 0xbf, 0x42, 0x2f, 0x49, 0x96, 0x05, 0x50, 0xa0, 0xad, 0x44, 0xec, 0x34, 0x46, 0xcb, 0x4c,
];
const AES_PUBKEY: [u8; 30] = [
    0x1d, 0x03, 0x0e, 0x80, 0xa1, 0x78, 0xdc, 0xee, 0xce, 0xcd, 0xa3, 0x77, 0xde, 0x12, 0x8d,
    0x8e, 0xd9, 0xdd, 0xcf, 0x55, 0xae, 0x61, 0xed, 0x46, 0xea, 0x12, 0x1a, 0x1c, 0xfc, 0x81,
];
const CRC_SALT: &[u8] = b"^j>WD3Kr?J2gLFjD4W2y@";
const MD5_SALT: &[u8] = b"Qclm8MGWUv59TnrR0XPg";
const APP_VERSION: &str = "36.2.28";

fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha1_hex(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(data);
    hex_lower(&h.finalize())
}

fn sha1_hex_upper(data: &[u8]) -> String {
    sha1_hex(data).to_uppercase()
}

fn md5_hex(data: &[u8]) -> String {
    let d = md5::compute(data);
    format!("{:x}", d)
}

/// 构造 k_ec 参数
fn make_k_ec(t: i64) -> String {
    use base64::Engine;
    let mut tok: Vec<u8> = Vec::with_capacity(48);
    tok.extend_from_slice(&AES_PUBKEY[..15]);
    tok.extend_from_slice(&[0x00, b's', 0x00, 0x00, 0x00]);
    tok.extend_from_slice(&(t as u32).to_le_bytes());
    tok.extend_from_slice(&AES_PUBKEY[15..]);
    tok.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00]);
    let mut crc_in = CRC_SALT.to_vec();
    crc_in.extend_from_slice(&tok);
    let crc = crc32fast::hash(&crc_in);
    tok.extend_from_slice(&crc.to_le_bytes());
    base64::engine::general_purpose::STANDARD.encode(&tok)
}

/// AES-128-CBC 加密(带 PKCS7)
fn aes_encrypt(plain: &[u8]) -> Vec<u8> {
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    type Enc = cbc::Encryptor<aes::Aes128>;
    // 注意: encrypt_padded_mut 会自己按 PKCS7 补位, 这里只预留空间, 不能手动补
    let mut buf = plain.to_vec();
    buf.resize(plain.len() + 16, 0);
    match Enc::new(&AES_KEY.into(), &AES_IV.into()).encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len()) {
        Ok(ct) => ct.to_vec(),
        Err(_) => Vec::new(),
    }
}

/// AES-128-CBC 解密。
///
/// 不用 `decrypt_padded_mut::<Pkcs7>`, 因为它对填充校验很严, 而 115 的响应
/// 尾部可能带非标准填充(参照样本: 140 字节里只有前 128 参与解密)。
/// 这里用 NoPadding 解密, 再手工判断并去掉合法的 PKCS7 尾巴。
fn aes_decrypt(cipher: &[u8]) -> Vec<u8> {
    use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    type Dec = cbc::Decryptor<aes::Aes128>;
    let n = cipher.len() & !15;
    if n == 0 {
        return Vec::new();
    }
    let mut buf = cipher[..n].to_vec();
    let plain = match Dec::new(&AES_KEY.into(), &AES_IV.into())
        .decrypt_padded_mut::<NoPadding>(&mut buf)
    {
        Ok(p) => p.to_vec(),
        Err(_) => return Vec::new(),
    };
    // 手工去 PKCS7: 末字节在 1..=16 且尾部连续相同才去掉
    let len = plain.len();
    if len > 0 {
        let pad = plain[len - 1] as usize;
        if (1..=16).contains(&pad) && pad <= len {
            if plain[len - pad..].iter().all(|b| *b as usize == pad) {
                return plain[..len - pad].to_vec();
            }
        }
    }
    plain
}

/// 115 的 LZ4 块格式: 2 字节小端长度 + 该长度的一块, 每块解压上限 0x2000
fn lz4_decompress_blocks(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 2 < src.len() {
        let len = src[i] as usize | ((src[i + 1] as usize) << 8);
        if len == 0 {
            break;
        }
        let start = i + 2;
        let end = (start + len).min(src.len());
        if end <= start {
            break;
        }
        match lz4_flex::block::decompress(&src[start..end], 0x2000) {
            Ok(mut d) => out.append(&mut d),
            Err(_) => break,
        }
        i = start + len;
    }
    out
}

/// Python 版 urlencode(sorted(items)) 的等价实现
fn urlencode_sorted(pairs: &[(String, String)]) -> String {
    fn quote_plus(s: &str) -> String {
        let mut out = String::new();
        for b in s.as_bytes() {
            match *b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(*b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
    let mut v: Vec<&(String, String)> = pairs.iter().collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.iter()
        .map(|(k, val)| format!("{}={}", quote_plus(k), quote_plus(val)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 取 upload_key(userkey)
pub async fn upload_key(st: &Client) -> Result<(String, String), String> {
    let ck = cookie(st)?;
    // userid 在 cookie 的 UID 字段
    // 注意: cookie 里的 UID 形如 "22099546_H3_1795246867",
    // 而签名的 userid 必须是**纯数字前缀**(p115client 里会 int(userid))
    let raw_uid = ck
        .split(';')
        .filter_map(|p| {
            let p = p.trim();
            p.strip_prefix("UID=").map(|s| s.to_string())
        })
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "cookie 里没有 UID".to_string())?;
    let userid: String = raw_uid
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if userid.is_empty() {
        return Err(format!("UID 里取不到数字前缀: {raw_uid}"));
    }
    let r = st
        .http
        .get("https://proapi.115.com/android/2.0/user/upload_key")
        .header("Cookie", &ck)
        .header("User-Agent", UA)
        .send()
        .await
        .map_err(|e| format!("upload_key 请求失败: {e}"))?;
    let body = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("upload_key 非 JSON({e}): {}", &body[..body.len().min(120)]))?;
    let uk = v
        .get("data")
        .and_then(|d| d.get("userkey"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if uk.is_empty() {
        return Err(format!("未取到 userkey: {}", &body[..body.len().min(140)]));
    }
    Ok((userid, uk))
}

/// 上传初始化(可能秒传)。返回解密后的 115 原始响应。
pub async fn upload_init(
    st: &Client,
    sha1: &str,
    filename: &str,
    filesize: i64,
    target_cid: &str,
) -> Result<Value, String> {
    let ck = cookie(st)?;
    let sha1 = sha1.trim().to_ascii_lowercase();
    if sha1.len() != 40 {
        return Err("sha1 必须是 40 位十六进制".into());
    }
    let (userid, userkey) = upload_key(st).await?;
    let target = format!("U_1_{}", if target_cid.trim().is_empty() { "0" } else { target_cid });
    let t = chrono::Utc::now().timestamp();

    // sig = SHA1(userkey + hex(SHA1(userid+fileid+target+"0")) + "000000") 大写
    let inner = sha1_hex(format!("{userid}{sha1}{target}0").as_bytes());
    let mut sig_in = userkey.clone().into_bytes();
    sig_in.extend_from_slice(inner.as_bytes());
    sig_in.extend_from_slice(b"000000");
    let sig = sha1_hex_upper(&sig_in);

    // token = MD5(MD5_SALT + f"{fileid}{filesize}{sign_key}{sign_val}{userid}{t}"
    //              + hex(MD5(str(int(userid)))) + appversion)
    let mut tok_in = MD5_SALT.to_vec();
    tok_in.extend_from_slice(format!("{sha1}{filesize}{}{}{userid}{t}", "", "").as_bytes());
    tok_in.extend_from_slice(md5_hex(userid.as_bytes()).as_bytes());
    tok_in.extend_from_slice(APP_VERSION.as_bytes());
    let token = md5_hex(&tok_in);

    let pairs: Vec<(String, String)> = vec![
        ("appid".into(), "0".into()),
        ("target".into(), target.clone()),
        ("sign_key".into(), String::new()),
        ("sign_val".into(), String::new()),
        ("topupload".into(), "true".into()),
        ("appversion".into(), APP_VERSION.into()),
        ("fileid".into(), sha1.clone()),
        ("filename".into(), filename.to_string()),
        ("filesize".into(), filesize.to_string()),
        ("userid".into(), userid.clone()),
        ("userkey".into(), userkey.clone()),
        ("t".into(), t.to_string()),
        ("sig".into(), sig.clone()),
        ("token".into(), token.clone()),
    ];
    let enc_src = urlencode_sorted(&pairs);
    let data = aes_encrypt(enc_src.as_bytes());
    let k_ec = make_k_ec(t);

    let r = st
        .http
        .post("https://uplb.115.com/4.0/initupload.php")
        .header("Cookie", &ck)
        .header(
            "User-Agent",
            format!("Mozilla/5.0 115disk/{APP_VERSION} 115Browser/{APP_VERSION} 115wangpan_android/{APP_VERSION}"),
        )
        .header("Content-Type", "application/x-www-form-urlencoded")
        .query(&[("k_ec", k_ec.as_str())])
        // 115 期望 body 就是原始密文字节(不是 data= 形式的 urlencode)
        .body(data.clone())
        .send()
        .await
        .map_err(|e| format!("initupload 请求失败: {e}"))?;
    let status = r.status().as_u16();
    let hdr_ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let hdr_ce = r
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let raw = r.bytes().await.map_err(|e| format!("读响应失败: {e}"))?;
    if raw.is_empty() {
        return Err(format!("initupload 空响应(HTTP {status})"));
    }
    let raw_len = raw.len();
    let raw_head: String = raw[..raw_len.min(24)]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    // 先试"明文 JSON"(出错时 115 可能直回 JSON)
    if let Ok(v0) = serde_json::from_slice::<Value>(&raw) {
        return Ok(json!({"http_status": status, "plain_json": true,
            "raw_len": raw_len, "raw_head": raw_head, "response": v0}));
    }
    // 诊断: 把各种可能的解码都试一遍, 找出能出 JSON 的那条
    let mut attempts: Vec<(&str, Vec<u8>)> = Vec::new();
    attempts.push(("raw", raw.to_vec()));
    attempts.push(("aes", aes_decrypt(&raw)));
    if raw.len() > 4 {
        attempts.push(("aes_skip4", aes_decrypt(&raw[4..])));
    }
    let mut found: Option<(&str, Value)> = None;
    for (name, bytes) in attempts.iter() {
        if bytes.is_empty() {
            continue;
        }
        // 直接 JSON?
        if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
            found = Some((*name, v));
            break;
        }
        // LZ4 块?
        let lz = lz4_decompress_blocks(bytes);
        if !lz.is_empty() {
            if let Ok(v) = serde_json::from_slice::<Value>(&lz) {
                found = Some((*name, v));
                break;
            }
        }
    }
    if let Some((how, v)) = found {
        return Ok(json!({"http_status": status, "decoded_by": how,
            "raw_len": raw_len, "raw_head": raw_head, "response": v}));
    }
    let decrypted = aes_decrypt(&raw);
    let dec_len = decrypted.len();
    let plain = if decrypted.is_empty() { raw.to_vec() } else { decrypted };
    let json_bytes = lz4_decompress_blocks(&plain);
    let json_bytes = if json_bytes.is_empty() { plain } else { json_bytes };
    let text = String::from_utf8_lossy(&json_bytes).to_string();
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok(json!({
            "http_status": status,
            "userid": userid,
            "target": target,
            "sig_prefix": &sig[..8],
            "response": v,
            "response_bytes": json_bytes.len(),
        })),
        Err(_) => Err(format!(
            "解密后仍不是 JSON | http={status} ct={hdr_ct} ce={hdr_ce} raw_len={raw_len} dec_len={dec_len} | head={raw_head} | 前60: {}",
            String::from_utf8_lossy(&json_bytes[..json_bytes.len().min(60)])
        )),
    }
}

fn percent_encode_bytes(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 3);
    for x in b {
        match *x {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*x as char)
            }
            _ => out.push_str(&format!("%{:02X}", x)),
        }
    }
    out
}

// ===================== 真实上传(3.0 凭证 + OSS POST + 115 回调登记) =====================

/// 上传一个内存里的文件到 115(新文件走普通上传; 已存在的 hash 会秒传失败)
///
/// 关键: 115 返回的 `callback` **本身就是 base64 字符串**, 必须原样透传给 OSS,
/// 再编码一次会被 OSS 拒(`callback configuration is not json format`)。
pub async fn upload_file(
    st: &Client,
    filename: &str,
    data: Vec<u8>,
    target_cid: &str,
) -> Result<Value, String> {
    let ck = cookie(st)?;
    let userid = ck
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("UID=").map(|s| s.to_string()))
        .next()
        .and_then(|s| {
            let n: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            if n.is_empty() { None } else { Some(n) }
        })
        .ok_or_else(|| "cookie 里没有可用的 UID".to_string())?;

    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(&data);
    let sha1_hex = hex_lower(&h.finalize());
    let size = data.len();
    let target = format!("U_1_{}", if target_cid.trim().is_empty() { "0" } else { target_cid });

    // 1) 取 OSS 凭证
    let part = reqwest::multipart::Part::text(userid.clone());
    let form = reqwest::multipart::Form::new()
        .part("userid", part)
        .text("filename", filename.to_string())
        .text("sha1", sha1_hex.clone())
        .text("size", size.to_string())
        .text("target", target.clone());
    let r = st
        .http
        .post("https://uplb.115.com/3.0/sampleinitupload.php")
        .header("Cookie", &ck)
        .header("User-Agent", format!("Mozilla/5.0 115disk/{APP_VERSION}"))
        .header("Referer", "https://115.com/")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("取上传凭证失败: {e}"))?;
    let init_body = r.text().await.unwrap_or_default();
    let init: Value = serde_json::from_str(&init_body)
        .map_err(|e| format!("凭证非 JSON({e}): {}", &init_body[..init_body.len().min(140)]))?;
    let g = |k: &str| -> String {
        init.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let (object, accessid, host, policy, signature, callback) = (
        g("object"), g("accessid"), g("host"), g("policy"), g("signature"), g("callback"),
    );
    if object.is_empty() || accessid.is_empty() || host.is_empty() {
        return Err(format!(
            "凭证不完整(status={}): {}",
            init.get("status").and_then(|v| v.as_i64()).unwrap_or(-1),
            &init_body[..init_body.len().min(200)]
        ));
    }
    let host = if host.starts_with("http") { host } else { format!("https://{host}") };

    // 2) POST 到 OSS, callback 原样透传
    let mut form = reqwest::multipart::Form::new()
        .text("key", object.clone())
        .text("policy", policy)
        .text("OSSAccessKeyId", accessid)
        .text("signature", signature)
        .text("success_action_status", "200");
    if !callback.is_empty() {
        form = form.text("callback", callback);
    }
    let file_part = reqwest::multipart::Part::bytes(data)
        .file_name(filename.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| format!("构造文件分片失败: {e}"))?;
    form = form.part("file", file_part);
    let r2 = st
        .http
        .post(&host)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("OSS 上传失败: {e}"))?;
    let status = r2.status().as_u16();
    let body = r2.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({"raw": body.clone()}));
    let ok = v.get("state").and_then(|x| x.as_bool()).unwrap_or(false) || status == 200;
    Ok(json!({
        "ok": ok,
        "http_status": status,
        "sha1": sha1_hex,
        "size": size,
        "object": object,
        "filename": filename,
        "target": target,
        "file_id": v.get("data").and_then(|d| d.get("file_id")).cloned().unwrap_or(Value::Null),
        "pick_code": v.get("data").and_then(|d| d.get("pick_code")).cloned().unwrap_or(Value::Null),
        "response": v,
    }))
}
