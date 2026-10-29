// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ecat_bench::{BenchResult, run_bench, run_bench_with_warmup};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

const USAGE: &str = "\
Usage: bench --url <URL> [--url <URL> ...] [options]

  --url <URL>         target URL (repeatable, at least one required)
  --concurrency N     concurrent workers (default 10)
  --requests N        measured requests per URL (default 500)
  --warmup N          warmup requests excluded from stats (default 50)
  --header \"N: V\"     request header (repeatable)
  --json              print one JSON object per target
  -h, --help          show this help

Exit status: 0 ok, 1 any failed request, 2 usage error.";

#[derive(Debug)]
struct Args {
    urls: Vec<String>,
    concurrency: usize,
    requests: u64,
    warmup: u64,
    headers: HeaderMap,
    json: bool,
}

#[derive(Debug)]
enum Parsed {
    Help,
    Run(Args),
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut urls = Vec::new();
    let mut headers = HeaderMap::new();
    let mut concurrency = 10usize;
    let mut requests = 500u64;
    let mut warmup = 50u64;
    let mut json = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--json" => json = true,
            "--url" => {
                let value = next_value(&mut args, "--url")?;
                urls.push(value);
            }
            "--concurrency" => {
                let value = next_value(&mut args, "--concurrency")?;
                concurrency = parse_num(&value, "--concurrency")?;
            }
            "--requests" => {
                let value = next_value(&mut args, "--requests")?;
                requests = parse_num(&value, "--requests")?;
            }
            "--warmup" => {
                let value = next_value(&mut args, "--warmup")?;
                warmup = parse_num(&value, "--warmup")?;
            }
            "--header" => {
                let raw = next_value(&mut args, "--header")?;
                let (name, value) = raw
                    .split_once(':')
                    .ok_or_else(|| format!("invalid header (expected \"Name: Value\"): {raw}"))?;
                let name = HeaderName::from_bytes(name.trim().as_bytes())
                    .map_err(|_| format!("invalid header name: {}", name.trim()))?;
                let value = HeaderValue::from_str(value.trim())
                    .map_err(|_| format!("invalid header value: {}", value.trim()))?;
                headers.append(name, value);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if urls.is_empty() {
        return Err("at least one --url is required".into());
    }
    if concurrency == 0 {
        return Err("--concurrency must be at least 1".into());
    }
    if requests == 0 {
        return Err("--requests must be at least 1".into());
    }

    Ok(Parsed::Run(Args {
        urls,
        concurrency,
        requests,
        warmup,
        headers,
        json,
    }))
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_num<T: std::str::FromStr>(value: &str, flag: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid number for {flag}: {value}"))
}

#[tokio::main]
async fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // 所有目标共用同一个 client，连接池跨目标复用；超时避免黑洞目标挂死。
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client");

    let mut any_failures = false;
    for url in &args.urls {
        let (result, failures, warmup_failures) = run_target(&client, url, &args).await;
        any_failures |= failures > 0;
        let successes = result.total_requests.saturating_sub(failures as u64);

        if args.json {
            println!(
                "{}",
                serde_json::json!({
                    "name": url,
                    "url": url,
                    "total_requests": result.total_requests,
                    "failures": failures,
                    "warmup_failures": warmup_failures,
                    "throughput_rps": result.throughput_rps,
                    "avg_latency_us": result.avg_latency_us,
                    "p50_latency_us": result.p50_latency_us,
                    "p95_latency_us": result.p95_latency_us,
                    "p99_latency_us": result.p99_latency_us,
                })
            );
        } else {
            result.print();
            println!("  failures:   {failures}");
            println!("  warmup_failures: {warmup_failures}");
            println!("  successes:  {successes}");
        }
    }

    if any_failures {
        std::process::exit(1);
    }
}

/// 预热单独跑一轮，失败单列：库的延迟统计已排除预热，若共用失败计数器，
/// 预热期的非 2xx 会虚增 failures 并让 successes 少算。
async fn run_target(
    client: &reqwest::Client,
    url: &str,
    args: &Args,
) -> (BenchResult, usize, usize) {
    let measured_failures = Arc::new(AtomicUsize::new(0));
    let warmup_failures = Arc::new(AtomicUsize::new(0));
    let measuring = Arc::new(AtomicBool::new(false));

    let request = {
        let client = client.clone();
        let url = url.to_string();
        let headers = args.headers.clone();
        let measured_failures = Arc::clone(&measured_failures);
        let warmup_failures = Arc::clone(&warmup_failures);
        let measuring = Arc::clone(&measuring);
        move || {
            let client = client.clone();
            let url = url.clone();
            let headers = headers.clone();
            let measured_failures = Arc::clone(&measured_failures);
            let warmup_failures = Arc::clone(&warmup_failures);
            let measuring = Arc::clone(&measuring);
            async move {
                match client.get(&url).headers(headers).send().await {
                    // 读完 body 以便连接回到池中复用
                    Ok(resp) if resp.status().is_success() => {
                        let _ = resp.bytes().await;
                    }
                    _ => {
                        let counter = if measuring.load(Ordering::Relaxed) {
                            &measured_failures
                        } else {
                            &warmup_failures
                        };
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    };

    if args.warmup > 0 {
        let _ = run_bench(url, args.concurrency, args.warmup, request.clone()).await;
    }
    measuring.store(true, Ordering::Relaxed);
    let result = run_bench_with_warmup(url, args.concurrency, args.requests, 0, request).await;
    (
        result,
        measured_failures.load(Ordering::Relaxed),
        warmup_failures.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    fn run_args(args: &[&str]) -> Args {
        match parse(args).unwrap() {
            Parsed::Run(args) => args,
            Parsed::Help => panic!("expected parsed args, got help"),
        }
    }

    #[test]
    fn defaults() {
        let args = run_args(&["--url", "http://127.0.0.1:8080/health"]);
        assert_eq!(args.urls, vec!["http://127.0.0.1:8080/health".to_string()]);
        assert_eq!(args.concurrency, 10);
        assert_eq!(args.requests, 500);
        assert_eq!(args.warmup, 50);
        assert!(args.headers.is_empty());
        assert!(!args.json);
    }

    #[test]
    fn repeatable_urls_and_headers() {
        let args = run_args(&[
            "--url",
            "http://a/health",
            "--url",
            "http://b/health",
            "--header",
            "X-Trace: abc",
            "--header",
            "Authorization: Bearer token",
            "--concurrency",
            "7",
            "--requests",
            "42",
            "--warmup",
            "3",
            "--json",
        ]);
        assert_eq!(
            args.urls,
            vec!["http://a/health".to_string(), "http://b/health".to_string()]
        );
        assert_eq!(args.headers.get("x-trace").unwrap(), "abc");
        assert_eq!(args.headers.get("authorization").unwrap(), "Bearer token");
        assert_eq!(args.concurrency, 7);
        assert_eq!(args.requests, 42);
        assert_eq!(args.warmup, 3);
        assert!(args.json);
    }

    #[test]
    fn missing_url_is_error() {
        let err = parse(&["--requests", "10"]).unwrap_err();
        assert!(err.contains("--url"), "unexpected error: {err}");
    }

    #[test]
    fn invalid_number_is_error() {
        let err = parse(&["--url", "http://x", "--requests", "nope"]).unwrap_err();
        assert!(err.contains("--requests"), "unexpected error: {err}");
    }

    #[test]
    fn help_exits_early() {
        assert!(matches!(parse(&["--help"]).unwrap(), Parsed::Help));
        assert!(matches!(parse(&["-h"]).unwrap(), Parsed::Help));
    }

    #[test]
    fn zero_counts_are_errors() {
        assert!(parse(&["--url", "http://x", "--concurrency", "0"]).is_err());
        assert!(parse(&["--url", "http://x", "--requests", "0"]).is_err());
        assert!(parse(&["--url", "http://x", "--warmup", "0"]).is_ok());
    }

    /// 回归：预热阶段的 5xx 不能计入 failures，也不能影响 measured successes。
    #[tokio::test]
    async fn warmup_failures_not_counted() {
        use axum::Router;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::routing::get;

        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/",
                get(|State(hits): State<Arc<AtomicUsize>>| async move {
                    // 前 2 个请求属于预热阶段，返回 500
                    if hits.fetch_add(1, Ordering::SeqCst) < 2 {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        StatusCode::OK
                    }
                }),
            )
            .with_state(Arc::clone(&hits));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{addr}/");
        let args = Args {
            urls: vec![url.clone()],
            concurrency: 1,
            requests: 4,
            warmup: 2,
            headers: HeaderMap::new(),
            json: false,
        };
        let (result, failures, warmup_failures) =
            run_target(&reqwest::Client::new(), &url, &args).await;

        assert_eq!(failures, 0, "warmup failures leaked into measured failures");
        assert_eq!(warmup_failures, 2, "warmup 5xx must be reported separately");
        assert_eq!(result.total_requests, 4);
        assert_eq!(result.total_requests - failures as u64, 4);
    }
}
