use reqwest;
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("Squid API Response Test");
    println!("========================\n");

    // Step 1: 获取API原始响应
    let url = "https://kraken.squid.wtf/track/?id=15200216&quality=LOSSLESS";
    println!("Testing API: {}\n", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    println!("Sending request...");
    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            println!("❌ Request failed: {}", e);
            return;
        }
    };

    println!("Status: {}", response.status());
    println!("Headers:");
    for (key, value) in response.headers() {
        println!("  {}: {:?}", key, value);
    }
    println!();

    // 获取原始响应文本
    let response_text = match response.text().await {
        Ok(text) => text,
        Err(e) => {
            println!("❌ Failed to read response: {}", e);
            return;
        }
    };

    println!("Response body:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 如果是JSON，尝试格式化显示
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&response_text) {
        let formatted = serde_json::to_string_pretty(&json_value).unwrap();
        println!("{}", formatted);

        // 尝试提取URL
        if let Some(url) = json_value.get("originalTrackUrl").and_then(|v| v.as_str()) {
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("\n✅ Found stream URL: {}", url);
        } else if let Some(url) = json_value.get("url").and_then(|v| v.as_str()) {
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("\n✅ Found stream URL (url field): {}", url);
        } else {
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("\n⚠️  No stream URL found in response");
            println!("Available fields:");
            if let Some(obj) = json_value.as_object() {
                for key in obj.keys() {
                    println!("  - {}", key);
                }
            }
        }
    } else {
        // 不是JSON，直接显示
        println!("{}", response_text);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("\n⚠️  Response is not JSON");
    }
}
