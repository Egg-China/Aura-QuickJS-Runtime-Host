use aura_quickjs_engine::{Limits, QuickJsRuntime};
use std::time::{Duration, Instant};

#[test]
fn evaluates_javascript_in_a_real_quickjs_context() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    let result = runtime
        .eval_int("40 + 2", Duration::from_secs(1))
        .expect("evaluate integer");
    assert_eq!(result, 42);
}

#[test]
fn runs_context_calls_with_an_absolute_deadline() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    let result = runtime
        .run_with_deadline(Instant::now() + Duration::from_secs(1), |context| {
            context.eval_int("6 * 7")
        })
        .expect("evaluate before deadline");
    assert_eq!(result, 42);
}

#[test]
fn interrupts_an_infinite_loop_at_the_deadline() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    let error = runtime
        .eval_int("for (;;) {}", Duration::from_millis(20))
        .expect_err("infinite loop must stop");
    assert_eq!(error.code(), "deadline-exceeded");
}

#[test]
fn enforces_the_heap_limit() {
    let limits = Limits {
        memory_bytes: 4 * 1024 * 1024,
        ..Limits::default()
    };
    let mut runtime = QuickJsRuntime::new(limits).expect("create constrained runtime");
    let error = runtime
        .eval_int(
            "const blocks = []; for (;;) blocks.push(new ArrayBuffer(1024 * 1024));",
            Duration::from_secs(2),
        )
        .expect_err("allocation must reach the heap limit");
    assert_eq!(error.code(), "resource-limit");
}

#[test]
fn enforces_the_stack_limit() {
    let limits = Limits {
        stack_bytes: 64 * 1024,
        ..Limits::default()
    };
    let mut runtime = QuickJsRuntime::new(limits).expect("create constrained runtime");
    let error = runtime
        .eval_int(
            "function recurse() { return 1 + recurse(); } recurse();",
            Duration::from_secs(1),
        )
        .expect_err("recursion must reach the stack limit");
    assert_eq!(error.code(), "resource-limit");
}

#[test]
fn reports_script_exceptions_without_exposing_the_message() {
    let mut runtime = QuickJsRuntime::new(Limits::default()).expect("create runtime");
    let error = runtime
        .eval_int(
            "throw new Error('payload-private-message')",
            Duration::from_secs(1),
        )
        .expect_err("script must fail");
    assert_eq!(error.code(), "evaluation-failed");
    assert_eq!(error.to_string(), "evaluation-failed");
}
