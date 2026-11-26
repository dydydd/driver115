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
