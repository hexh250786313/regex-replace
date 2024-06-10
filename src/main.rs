extern crate atty;

use atty::Stream;
use clap::Parser;
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process;
use tempfile::NamedTempFile;

const NEW_LINES: [&str; 7] = [
    "\\n",
    "\\r",
    "\\r\\n",
    "\\u{000B}",
    "\\u{000C}",
    "\\u{2028}",
    "\\u{2029}",
];

#[derive(Parser)]
struct Args {
    ///
    /// 指定的目录，默认为当前目录
    ///
    #[clap(short = 'd', long = "directory", conflicts_with = "files")]
    directory: Option<PathBuf>,
    ///
    /// 指定的文件，可以指定多个
    ///
    #[clap(short = 'f', long = "files", conflicts_with = "directory", value_delimiter = ' ', num_args = 1..)]
    files: Option<Vec<PathBuf>>,
    ///
    /// 查询正则
    ///
    #[clap(short = 'p', long = "pattern")]
    pattern: String,
    ///
    /// 替换字符串
    ///
    #[clap(short = 'r', long = "replacement")]
    replacement: String,
}

///
/// 行读取器
///
struct LineReader {
    lines: Box<dyn Iterator<Item = io::Result<String>>>,
}

impl LineReader {
    fn new(reader: Box<dyn BufRead>) -> Self {
        Self {
            lines: Box::new(reader.lines()),
        }
    }

    fn read_lines(&mut self, num_lines: usize) -> io::Result<Vec<String>> {
        let mut lines = Vec::new();
        for line in self.lines.by_ref().take(num_lines) {
            lines.push(line?);
        }
        Ok(lines)
    }
}

///
/// 用逐行的方法替换文件
///
fn replace_in_file_line_by_line(
    target_file: &PathBuf,
    re: &Regex,
    replacement: &str,
    max_line_number: &usize,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    //
    // 创建临时文件
    //
    let temp_file = NamedTempFile::new()?;
    let temp_file_path = temp_file.path().to_path_buf();
    let file = OpenOptions::new()
        .append(true)
        .open(temp_file_path.clone())?;
    let mut file = BufWriter::new(file);

    let f = File::open(target_file)?;
    let reader = BufReader::new(f);
    let mut line_reader = LineReader::new(Box::new(reader));

    //
    // 先读取若干行
    // 单行正则匹配的话，逐行处理
    // 多行的话，预加载两倍的行数防止跨行匹配失败
    //
    let mut buffer_lines = line_reader.read_lines(if *max_line_number == 1 {
        1
    } else {
        *max_line_number * 2
    })?;

    loop {
        if buffer_lines.is_empty() {
            break;
        }
        let buffer_text = &buffer_lines.join("\n");
        let buffer_text_replaced = re.replace_all(buffer_text, replacement);

        //
        // 在多行匹配情况下，如果再次用正则匹配可以匹配到结果，说明不可以使用逐行匹配
        // 例如：" \n " -> " \n  "
        // 这时候，应该抛出错误，
        // 然后换用整个文件替换的方式
        //
        if *max_line_number > 1 && re.is_match(&buffer_text_replaced) {
            return Err(
                "Cross-line match found, please use the whole file replacement method".into(),
            );
        }

        //
        // buffer_text_replaced 转换为字符串 Vec
        //
        let buffer_lines_replaced = buffer_text_replaced
            .split('\n')
            .map(String::from)
            .collect::<Vec<_>>();
        //
        // 把这个 Vec 分成两部分，分别是后 n 行，和前面 len() - n 行
        // 计算分割线索引
        //
        let split_at = if buffer_lines_replaced.len() > *max_line_number {
            buffer_lines_replaced.len() - max_line_number
        } else {
            0
        };
        //
        // 切开两部分
        //
        let (processed_part, unprocessed_part) = if *max_line_number == 1 {
            //
            // 单行的情况下，不需要分割，防止重复处理匹配，例如可能会出现以下情况
            // 替换单个空格 " " 为两个空格 "  "
            // 如果把已经处理过空格的 unprocessed_part 移入下一次循环，会导致重复处理
            //
            (buffer_lines_replaced.as_slice(), &[][..])
        } else {
            buffer_lines_replaced.split_at(split_at)
        };
        //
        // 把已经完全处理完毕的部分写入临时文件
        //
        for line in processed_part {
            writeln!(file, "{}", line)?;
        }
        buffer_lines.clear();
        //
        // 未完全处理的部分并入下一次的循环
        //
        let last_lines = unprocessed_part.iter().map(|s| s.to_string());
        //
        // 读取接下来 n 行
        // 如果为空，说明没有后续内容，则把剩余部分写入文件，结束循环
        // 如果不为空，则继续循环
        //
        let next = line_reader.read_lines(*max_line_number)?;
        if next.is_empty() {
            for line in last_lines {
                write!(file, "{}", line)?;
            }
            break;
        } else {
            buffer_lines.extend(last_lines);
            buffer_lines.extend(next);
        }
    }
    file.flush()?;

    //
    // Persist the temp file
    //
    let _ = temp_file.persist(&temp_file_path)?;

    Ok(temp_file_path)
}

///
/// 直接替换整个文件
///
fn replace_in_file_whole_file(
    target_file: &PathBuf,
    re: &Regex,
    replacement: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    //
    // 创建临时文件
    //
    let temp_file = NamedTempFile::new()?;
    let temp_file_path = temp_file.path().to_path_buf();
    let file = OpenOptions::new()
        .append(true)
        .open(temp_file_path.clone())?;
    let mut file = BufWriter::new(file);

    //
    // 读取整个文件
    //
    let contents = fs::read_to_string(target_file)?;
    //
    // 替换内容
    //
    let replaced_contents = re.replace_all(&contents, replacement);
    write!(file, "{}", replaced_contents)?;

    file.flush()?;
    let _ = temp_file.persist(&temp_file_path)?;

    Ok(temp_file_path)
}

///
/// 替换文件内容
///
fn replace_in_file(
    target_file: &PathBuf,
    re: &Regex,
    replacement: &str,
    max_line_number: &usize,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let temp_file_path =
        // match replace_in_file_whole_file(target_file, re, replacement) {
        //     Ok(temp_file_path) => temp_file_path,
        //     Err(_) => replace_in_file_line_by_line(target_file, re, replacement, max_line_number)?,
        // };
        match replace_in_file_line_by_line(target_file, re, replacement, max_line_number) {
            Ok(temp_file_path) => temp_file_path,
            Err(_) => replace_in_file_whole_file(target_file, re, replacement)?,
        };

    Ok(temp_file_path)
}

///
/// 检查字符串是否包含有效的转义序列
/// 对于单个反斜杠，默认情况下会被 rust 忽略处理
/// 但是这里选择直接报错，必须确保输入的正则是完全正确的
///
fn check_string(s: &str) -> Result<(), String> {
    // 有点问题，先注释
    // let mut chars = s.chars().peekable();
    // while let Some(ch) = chars.next() {
    //     if ch == '\\' {
    //         match chars.peek() {
    //             Some('n') | Some('r') | Some('t') | Some('\'') | Some('"') => {
    //                 let _ = chars.next();
    //             }
    //             _ => return Err(format!("存在多余的单斜杠: {}", s)),
    //         }
    //     }
    // }
    Ok(())
}

fn walk_directory(dir: &PathBuf) -> Vec<PathBuf> {
    let walker = WalkBuilder::new(dir).git_ignore(true).build();

    let mut files = Vec::new();

    for result in walker {
        match result {
            Ok(entry) => {
                let path = entry.into_path();
                if path.is_file() {
                    files.push(path);
                }
            }
            Err(err) => {
                eprintln!("错误: {}", err);
                process::exit(1);
            }
        }
    }

    files
}

impl Args {
    fn parse_args() -> Self {
        let mut args = Self::parse();
        if args.directory.is_none() && args.files.is_none() {
            args.directory = Some(env::current_dir().unwrap());
        }
        if args.files.is_none() {
            args.files = None
        }
        args.validate_paths();
        args
    }

    fn validate_paths(&self) {
        if let Some(dir) = &self.directory {
            if !dir.exists() {
                eprintln!("错误: 目录 {:?} 不存在", dir);
                process::exit(1);
            }
            if !dir.is_dir() {
                eprintln!("错误: {:?} 不是一个目录", dir);
                process::exit(1);
            }
        }

        if let Some(files) = &self.files {
            for file in files {
                if !file.exists() {
                    eprintln!("错误: 文件 {:?} 不存在", file);
                    process::exit(1);
                }
                if !file.is_file() {
                    eprintln!("错误: {:?} 不是一个文件", file);
                    process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let args = Args::parse_args();

    let mut files = Vec::new();

    //
    // 管道输入，接受的是一个文件路径列表
    //
    if !atty::is(Stream::Stdin) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let file_path = PathBuf::from(line.unwrap());
            files.push(file_path);
        }
    } else {
        if args.directory.is_some() {
            if let Some(directory) = &args.directory {
                files.extend(walk_directory(directory));
            }
        }

        if let Some(file_paths) = &args.files {
            files.extend(file_paths.iter().cloned());
        }
    }

    let count = NEW_LINES
        .iter()
        .map(|newline| args.pattern.matches(newline).count())
        .sum::<usize>();

    //
    // 最大行数
    // 正则跨行匹配，不允许超过 n + 1 行
    // 否则这个函数会失效
    //
    let max_line_number: usize = count + 1;

    let replacement = match unescape::unescape(&args.replacement) {
        Some(replacement) => replacement,
        None => {
            eprintln!("错误: 目标字符串转义失败");
            process::exit(1);
        }
    };
    let replacement = replacement.as_str();

    match Regex::new(&args.pattern) {
        Ok(re) => match check_string(&args.pattern) {
            Ok(_) => {
                let temp_files: Vec<_> = files
                    .par_iter()
                    .filter_map(|file| {
                        match replace_in_file(file, &re, replacement, &max_line_number) {
                            Ok(temp_file) => Some((file.clone(), temp_file)),
                            Err(err) => {
                                eprintln!("处理文件错误 {:?}: {}", file, err);
                                None
                            }
                        }
                    })
                    .collect();

                for (file, temp_file) in temp_files {
                    let metadata = match fs::metadata(&file) {
                        Ok(metadata) => metadata,
                        Err(err) => {
                            eprintln!("获取元信息错误 {:?}: {}", file, err);
                            process::exit(1);
                        }
                    };
                    if let Err(err) = fs::set_permissions(&temp_file, metadata.permissions()) {
                        eprintln!("设置文件权限错误 {:?}: {}", temp_file, err);
                        process::exit(1);
                    }
                    if let Err(err) = fs::copy(&temp_file, &file) {
                        eprintln!("复制文件错误 {:?}: {}", file, err);
                        process::exit(1);
                    }
                    if let Err(err) = fs::remove_file(&temp_file) {
                        eprintln!("删除临时文件错误: {}", err);
                        process::exit(1);
                    }
                }
            }
            Err(err) => {
                eprintln!("错误: {}", err);
                process::exit(1);
            }
        },
        Err(err) => {
            eprintln!("错误: 无效正则表达式: {}", err);
            process::exit(1);
        }
    }
}
