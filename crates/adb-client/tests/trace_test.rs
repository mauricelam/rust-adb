use adb_client::trace::{trace_enabled, trace_init, TraceCategory};

#[test]
fn test_trace_category_parsing() {
    let errs = trace_init("adb,sockets,unknown_category");
    assert!(trace_enabled(TraceCategory::Adb));
    assert!(trace_enabled(TraceCategory::Sockets));
    assert!(!trace_enabled(TraceCategory::Packets));
    assert_eq!(errs, vec!["Unknown trace flag: unknown_category"]);
}

#[test]
fn test_trace_all_category() {
    trace_init("all");
    assert!(trace_enabled(TraceCategory::Adb));
    assert!(trace_enabled(TraceCategory::Sockets));
    assert!(trace_enabled(TraceCategory::Packets));
    assert!(trace_enabled(TraceCategory::Transport));
    assert!(trace_enabled(TraceCategory::Services));
}

#[test]
fn test_trace_empty() {
    trace_init("");
    assert!(!trace_enabled(TraceCategory::Adb));
    assert!(!trace_enabled(TraceCategory::Sockets));
}
