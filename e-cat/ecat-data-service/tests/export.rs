use ecat_data_service::export::{csv_of_points, xlsx_of_points};
use ecat_data_service::models::HistoryPoint;
use serde_json::json;

#[test]
fn csv_has_header_and_escapes_commas() {
    let points = vec![
        HistoryPoint {
            ts: 1690000000000,
            value: json!(23.5),
        },
        HistoryPoint {
            ts: 1690000000001,
            value: json!("a,b\"c"),
        },
    ];
    let csv = csv_of_points(&points);
    assert!(
        csv.starts_with("\u{FEFF}ts,value\n"),
        "缺表头(含 UTF-8 BOM): {csv}"
    );
    assert!(csv.contains("1690000000000,23.5\n"));
    assert!(csv.contains("\"a,b\"\"c\"\n"), "逗号/引号未转义: {csv}");
}

#[test]
fn csv_neutralizes_formula_prefixes_in_strings() {
    for (raw, expected) in [
        ("=1+1", "'=1+1"),
        ("+cmd", "'+cmd"),
        ("-cmd", "'-cmd"),
        ("@SUM(A1)", "'@SUM(A1)"),
        ("\tcmd", "'\tcmd"),
        ("\rcmd", "\"'\rcmd\""),
    ] {
        let points = vec![HistoryPoint {
            ts: 1,
            value: json!(raw),
        }];
        let csv = csv_of_points(&points);
        assert!(
            csv.contains(&format!("1,{expected}\n")),
            "raw={raw:?} csv={csv}"
        );
    }
    // 数值（含负数）不是字符串，不能被误加前缀
    let csv = csv_of_points(&[HistoryPoint {
        ts: 2,
        value: json!(-23.5),
    }]);
    assert!(csv.contains("2,-23.5\n"), "csv={csv}");
    // 公式前缀 + 逗号：先加 `'` 再按 RFC4180 引号包裹
    let csv = csv_of_points(&[HistoryPoint {
        ts: 3,
        value: json!("=a,b"),
    }]);
    assert!(csv.contains("3,\"'=a,b\"\n"), "csv={csv}");
}

#[test]
fn xlsx_produces_valid_zip() {
    let points = vec![
        HistoryPoint {
            ts: 1690000000000,
            value: json!(23.5),
        },
        HistoryPoint {
            ts: 1690000000001,
            value: json!("hot"),
        },
    ];
    let buf = xlsx_of_points(&points).unwrap();
    // xlsx = zip 容器，魔数 PK\x03\x04
    assert_eq!(&buf[0..4], b"PK\x03\x04");
}
