use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;

fn main() {
    simple_playback_test();
}

#[allow(dead_code)]
fn simple_playback_test() {
    // 最简单的播放测试 - 直接播放缓存文件
    let file_path = "/Users/yanyao/Library/Caches/dab/363236463.mp3";

    println!("正在播放: {}", file_path);

    // 打开文件
    let file = File::open(file_path).expect("无法打开文件");
    let buf_reader = BufReader::new(file);

    // 创建输出流和解码器
    let (_stream, stream_handle) = OutputStream::try_default().expect("无法初始化音频输出");
    let sink = Sink::try_new(&stream_handle).expect("无法创建音频sink");
    let source = Decoder::new(buf_reader).expect("无法解码MP3");

    // 播放
    sink.append(source);
    sink.set_volume(0.8);

    println!("播放中... 按 Ctrl+C 停止");
    println!("请仔细听是否有pop杂音");

    // 阻塞直到播放完成
    sink.sleep_until_end();

    println!("播放完成");
}

// 更详细的测试，包含多次播放
#[allow(dead_code)]
fn detailed_playback_test() {
    let file_path = "/Users/yanyao/Library/Caches/dab/363236463.mp3";

    println!("详细播放测试");
    println!("=============");

    for i in 0..3 {
        println!("\n第 {} 次播放:", i + 1);

        // 每次创建新的资源
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                println!("打开文件失败: {}", e);
                return;
            }
        };

        let (_stream, stream_handle) = match OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                println!("音频初始化失败: {}", e);
                return;
            }
        };

        let sink = Sink::try_new(&stream_handle).unwrap();
        let buf_reader = BufReader::new(file);
        let source = match Decoder::new(buf_reader) {
            Ok(s) => s,
            Err(e) => {
                println!("解码失败: {}", e);
                return;
            }
        };

        sink.append(source);
        sink.set_volume(0.8);

        // 播放10秒
        println!("播放10秒...");
        std::thread::sleep(std::time::Duration::from_secs(10));

        // 停止并清理
        sink.stop();
        drop(sink);
        drop(_stream);

        println!("本次播放结束");

        if i < 2 {
            println!("等待2秒后继续...");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    println!("\n测试完成！");
    println!("如果每次播放都有pop音，说明问题可能在音频文件本身或rodio");
    println!("如果没有pop音，说明问题在DAB的流式处理逻辑中");
}
