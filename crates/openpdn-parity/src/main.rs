//! `openpdn-parity <script> <outdir> [--bench N]`
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: openpdn-parity <script> <outdir> [--bench N] | --compare <dirA> <dirB>");
        std::process::exit(2);
    }
    if args[1] == "--compare" {
        std::process::exit(compare(&args[2], &args[3]));
    }
    let mut bench = 1;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "--bench" {
            bench = args[i + 1].parse().unwrap();
            i += 1;
        }
        i += 1;
    }
    let text = std::fs::read_to_string(&args[1]).expect("read script");
    std::fs::create_dir_all(&args[2]).unwrap();
    let mut times = Vec::new();
    for run in 0..bench {
        let t = Instant::now();
        let mut it = openpdn_parity::Interp::new(&args[2], run == bench - 1);
        it.base_dir = std::path::Path::new(&args[1]).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if let Err(e) = it.run(&text) {
            eprintln!("{}: {e}", args[1]);
            std::process::exit(1);
        }
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    if bench > 1 {
        times.sort_by(f64::total_cmp);
        let name = std::path::Path::new(&args[1]).file_name().unwrap().to_string_lossy();
        println!("{name}: median {:.2} ms, min {:.2} ms ({bench} runs)", times[times.len() / 2], times[0]);
    }
}

fn compare(a: &str, b: &str) -> i32 {
    let mut failures = 0;
    let mut n = 0;
    let mut names: Vec<_> = std::fs::read_dir(a).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
    names.sort();
    for name in names {
        let lossy = name.to_string_lossy();
        if !(lossy.ends_with(".bin") || lossy.ends_with(".txt")) {
            continue; // e.g. saved documents, whose encoders legitimately differ
        }
        let pa = std::path::Path::new(a).join(&name);
        let pb = std::path::Path::new(b).join(&name);
        n += 1;
        let res = if name.to_string_lossy().ends_with(".bin") {
            openpdn_parity::compare_dumps(&pa, &pb)
        } else {
            match (std::fs::read(&pa), std::fs::read(&pb)) {
                (Ok(x), Ok(y)) if x == y => Ok(()),
                (Ok(_), Ok(_)) => Err("text differs".into()),
                (_, Err(e)) | (Err(e), _) => Err(e.to_string()),
            }
        };
        if let Err(e) = res {
            failures += 1;
            println!("DIFF {}: {e}", name.to_string_lossy());
        }
    }
    println!("{} of {n} outputs identical", n - failures);
    i32::from(failures > 0)
}
