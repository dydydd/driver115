# driver115

115.com 网盘的纯 Rust 客户端，从 115driver 的 Rust 重写版中拆出。

**只用 cookie 鉴权，不依赖 Python、不依赖 p115client、不需要 OAuth App。**

## 能力

| 功能 | 说明 |
|---|---|
| 列目录 | `proapi` 优先、`webapi` 兜底，`cid="0"` 为根目录 |
| 网盘内搜索 | `webapi/files/search` |
| 分享 | 解析分享链接、快照、转存（`webapi/share/snap` / `share/receive`） |
| 离线下载 | 添加任务（lixian 接口）、任务列表、删除、目标文件夹 |
| 批量操作 | 移动、删除（进回收站） |
| 扫码登录 | 申请二维码（PNG base64）、轮询状态、SSO 换 cookie |
| **秒传 / 普通上传** | 115 私有签名完整移植：`sig`/`token`/`k_ec` + AES-128-CBC + CRC32 + LZ4 + OSS 回调 |

压缩包密码、分享码解析、文件大小格式化等辅助函数也都在内。

## 用法

```rust
use driver115::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 直接从 115driver 的 config.yaml 读取 settings_115[0]
    let client = Client::from_yaml("/path/to/config.yaml")?;

    // 或者手动给 cookie
    // let client = Client::new("UID=...; CID=...; SEID=...");

    let dirs = driver115::driver::list_dirs(&client, "0").await?;
    for d in dirs {
        println!("{}  {}", d.id, d.name);
    }
    Ok(())
}
```

更完整的调用示例见 `examples/list.rs`。

## 上传流程说明

```
1) POST uplb.115.com/3.0/sampleinitupload.php
   → object / accessid / host / policy / signature / callback
2) POST <host>  (multipart:
   key / policy / OSSAccessKeyId / signature /
   callback=115返回的base64串(原样透传) / file)
   → 115 回调登记, 返回 file_id 与 pickcode
```

**关键坑（已处理）**：
- 115 的 `callback` 字段本身就是 base64 字符串，再编码一次会被 OSS 拒绝
- 签名用的 `userid` 必须是 cookie 里 `UID=数字_xxx` 的**纯数字前缀**
- 响应里只有前 N 字节是 AES 密文，需宽松去 PKCS7 填充

## 加密实现

115 所谓 "ECDH" 实际是固定 key/iv 的 AES-128-CBC。常量与算法来自
`p115cipher.make_upload_payload` 的等价 Rust 移植：

```
sig   = SHA1(userkey || hex(SHA1(userid+fileid+target+"0")) || "000000")  大写
token = MD5(MD5_SALT || f"{fileid}{filesize}{sign_key}{sign_val}{userid}{t}"
            || hex(MD5(str(int(userid)))) || appversion)
k_ec  = base64(pubkey[:15] + \x00s\x00\x00\x00 + le32(t) + pubkey[15:]
                + \x00\x01\x00\x00\x00 + le32(crc32(CRC_SALT+token)))
data  = AES-128-CBC(urlencode(sorted(payload)))
```

## 许可

MIT


## Open API (115 开放平台)

除 cookie 驱动外, 还实现了 `proapi.115.com/open/*` 的 Open API 客户端:

```rust
use driver115::{OpenClient, OpenCreds};

// 凭证由调用方传入, 本库不内置 app_id / token
let creds = OpenCreds {
    app_id: Some(你的_app_id),
    access_token: 你的_access_token.into(),
    refresh_token: 你的_refresh_token.into(),
    ..Default::default()
};
let api = OpenClient::new(creds);
let files = api.fs_files("0", 100, 0).await?;
let hits = api.fs_search("关键词", "0", 100, 0).await?;
```

覆盖的接口:

| 分类 | 方法 |
|---|---|
| 认证 | 设备码二维码 / 扫码换 token / 刷新 token（PKCE, verifier 默认 64 个 0） |
| 文件 | 列目录 / 搜索 / 目录信息 / 移动 / 复制 / 删除 / 改名 / 下载直链 |
| 离线 | 添加 URL / 添加 BT / 任务列表 / 配额 / 删除 / 清空 |
| 回收站 | 列表 / 删除 / 还原 |
| 上传 | init / get_token |
| 其他 | 视频播放 / 字幕 / VIP 二维码 |

**没有 appid/token 也能编译和使用全部接口**: 构造 `OpenCreds::default()` 即可,
请求会发到 115 服务端并原样返回鉴权错误, 方便调试与对接自己的开放平台应用。

示例: `cargo run --example open`

