use clap::Parser;
use ignore::WalkBuilder;
use regex::Regex;
use std::env;
use std::path::PathBuf;
use std::process;

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
/// 检查字符串是否包含有效的转义序列
/// 对于单个反斜杠，默认情况下会被 rust 忽略处理
/// 但是这里选择直接报错，必须确保输入的正则是完全正确的
///
fn check_string(s: &str) -> Result<(), String> {
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('n') | Some('r') | Some('t') | Some('\\') | Some('\'') | Some('"') => {
                    //
                    // 这是一个有效的转义序列，忽略它
                    //
                    let _ = chars.next();
                }
                _ => return Err(format!("Invalid escape sequence in string: {}", s)),
            }
        }
    }
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
                eprintln!("Error: {}", err);
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
                eprintln!("Error: Directory {:?} does not exist", dir);
                process::exit(1);
            }
            if !dir.is_dir() {
                eprintln!("Error: {:?} is not a directory", dir);
                process::exit(1);
            }
        }

        if let Some(files) = &self.files {
            for file in files {
                if !file.exists() {
                    eprintln!("Error: File {:?} does not exist", file);
                    process::exit(1);
                }
                if !file.is_file() {
                    eprintln!("Error: {:?} is not a file", file);
                    process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let args = Args::parse_args();

    if args.directory.is_some() {
        if let Some(directory) = &args.directory {
            let files = walk_directory(directory);
            // println!("Files: {:?}", files);
        }
    }

    if args.files.is_some() {
        // println!("Files: {:?}", args.files);
    }

    // println!("Pattern: {}", args.pattern);
    // println!("Replacement: {}", args.replacement);

    println!("正则字符 {}", args.pattern);

    // let re = Regex::new(&args.pattern).unwrap();
    //
    match Regex::new(&args.pattern) {
        Ok(re) => match check_string(&args.pattern) {
            Ok(_) => {
                if re.is_match(&args.replacement) {
                    println!("The string matches the pattern");
                } else {
                    println!("The string does not match the pattern");
                }
            }
            Err(err) => {
                eprintln!("Error: {}", err);
            }
        },
        Err(err) => {
            eprintln!("Error: Invalid regular expression: {}", err);
            process::exit(1);
        }
    }
}
