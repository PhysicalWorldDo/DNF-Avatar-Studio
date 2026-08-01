use std::path::PathBuf;

use dnf_avatar_studio_lib::resource_probe::{parse_resource_probe_args, probe_avatar_resources};

const ORIGINAL_NPK_DIR: &str = r"E:\DNF游戏原始NPK";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let options = parse_resource_probe_args(std::env::args_os())?;
    if options.help {
        println!("{}", help_text());
        return Ok(());
    }
    if let Some(data_dll) = options.data_dll {
        std::env::set_var("DNF_AVATAR_DATA_DLL", data_dll);
    }
    let game_dir = options.inspect_npk.then(|| {
        options.game_dir.unwrap_or_else(|| {
            let original = PathBuf::from(ORIGINAL_NPK_DIR);
            if original.is_dir() {
                original
            } else {
                PathBuf::from(dnf_avatar_studio_lib::game_resolver::DEFAULT_GAME_DIR)
            }
        })
    });
    let report = probe_avatar_resources(
        &options.queries,
        game_dir.as_deref(),
        options.frame_index,
        options.limit,
    )?;
    if report.items.is_empty() {
        return Err(format!("没有找到：{}", options.queries.join("、")));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("序列化排查报告失败：{error}"))?
    );
    Ok(())
}

fn help_text() -> &'static str {
    r#"Avatar Resource Probe - 时装资源快速排查

用法：
  avatar_resource_probe.exe [选项] <物品ID或完整/部分名称> [...]

示例：
  avatar_resource_probe.exe 415560060 415560061 415560062 415560063
  avatar_resource_probe.exe "星辰命运银色长发" --game-dir E:\DNF游戏原始NPK
  avatar_resource_probe.exe 101500882 --no-npk

选项：
  --data-dll <路径>  指定 avatar_data.dll；默认读取工具同目录文件
  --game-dir <目录>  指定 ImagePacks2/NPK 目录
  --frame <序号>     解码并计算该帧的 SHA-256，默认 0
  --limit <数量>     最多返回 1..100 件，默认 20
  --no-npk           只查询 DLL，不打开 NPK/IMG
  -h, --help         显示帮助

报告会区分 mask 的“EQU 已授权”和“当前默认激活”。没有非默认实际染色值时，
[enable dye] 不会让普通预览自动叠加 mask。单个坏资源会记录到该图层 errors 并继续；
NPK 缓存与 IMG 解码量均有安全上限。"#
}
