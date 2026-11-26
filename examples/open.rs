//! Open API 示例: 凭证由调用方提供, 这里演示没有凭证时的行为。
use driver115::{OpenClient, OpenCreds};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 没有 appid / access_token 也能构造客户端; 接口会原样返回 115 的鉴权错误。
    let client = OpenClient::new(OpenCreds::default());

    match client.user_info().await {
        Ok(d) => println!("用户信息: {d}"),
        Err(e) => println!("按预期失败(缺凭证): {e}"),
    }

    // 有凭证时:
    // let creds = OpenCreds { app_id: Some(你的appid), access_token: "你的token".into(), ..Default::default() };
    // let client = OpenClient::new(creds);
    // let files = client.fs_files("0", 100, 0).await?;
    Ok(())
}
