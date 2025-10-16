use dab::config::Config;
use std::env;
use std::process::Command;

fn main() {
    println!("Testing Download-Complete Mode Fix");
    println!("===================================\n");

    // Check if streaming_buffer is set to 0
    if let Ok(val) = env::var("DAB_STREAMING_BUFFER") {
        println!("DAB_STREAMING_BUFFER={}", val);
        if val == "0" {
            println!("✅ Download-complete mode is enabled");
            println!("This should download the entire file before playback");
            println!("There should be NO pop sounds\n");
        }
    } else {
        println!("⚠️ DAB_STREAMING_BUFFER not set");
        println!("Run with: DAB_STREAMING_BUFFER=0 cargo run --bin test_download_complete\n");
    }

    // Load config to verify
    let config = Config::load();
    println!("Config streaming_buffer: {} MB", config.streaming_buffer);

    if config.streaming_buffer == 0 {
        println!("\n✅ Configuration confirms download-complete mode");
        println!("Expected behavior:");
        println!("1. Track will be completely downloaded before playback");
        println!("2. No StreamingAudioSource will be used");
        println!("3. A seekable file handle will be returned");
        println!("4. NO pop sounds should occur");
    } else {
        println!(
            "\n⚠️ Streaming mode is enabled (streaming_buffer = {} MB)",
            config.streaming_buffer
        );
        println!("This may still have pop sounds for high-bitrate content");
    }

    println!("\nKey changes in the fix:");
    println!("- loader.rs: When streaming_buffer=0, downloads complete file first");
    println!("- Returns LoadResult::Seekable instead of LoadResult::Streaming");
    println!("- Bypasses StreamingAudioSource entirely");
    println!("- Uses simple file handle for playback (no sync_read_fallback issues)");

    println!("\nTo test:");
    println!("1. Run: DAB_STREAMING_BUFFER=0 cargo run --release");
    println!("2. Search and play a LOSSLESS track");
    println!("3. Watch logs for 'Download-complete mode enabled'");
    println!("4. Listen for pops (should be NONE)");
}
