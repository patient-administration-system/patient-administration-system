//! RCA-IN-2, RCA-TEST-8: a figure outside its range is refused, naming the figure and the range, and is never
//! moved to the nearest bound. Needs PostgreSQL: `cargo test -- --ignored`.
mod common;
use patient_administration_system::Error;

fn refused(result: Result<impl std::fmt::Debug, Error>, figure: &str, range: &str) {
    match result {
        Err(Error::Validation(m)) => {
            assert!(m.contains(figure), "{m}");
            assert!(m.contains(range), "{m}");
        }
        other => panic!("{figure}: expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_look_back_windows_refuse_a_value_outside_their_range() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    for days in [0, -1, 366, i64::MAX] {
        refused(s.tolerance.status(days).await, "days", "1 to 365");
    }
    for days in [1, 365] {
        assert!(s.tolerance.status(days).await.is_ok(), "{days}");
    }
    for days in [0, 1826, i64::MIN] {
        refused(
            s.noshow.fairness_audit(days, None).await,
            "days",
            "1 to 1825",
        );
    }
    for days in [6, 1826, 0] {
        refused(s.noshow.evaluation(days, None).await, "days", "7 to 1825");
    }
    assert!(s.noshow.evaluation(7, None).await.is_ok());
    assert!(s.noshow.fairness_audit(1825, None).await.is_ok());
    for days in [0, 3651, -5] {
        refused(
            s.receipts.contact_problems(days, 10_i64).await,
            "since_days",
            "1 to 3650",
        );
    }
    assert!(s.receipts.contact_problems(3650, 10_i64).await.is_ok());
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_print_queue_refuses_a_limit_outside_its_range() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    for limit in [0, 501, -1] {
        refused(s.printing.queue(limit).await, "limit", "1 to 500");
    }
    assert!(s.printing.queue(1).await.is_ok());
    assert!(s.printing.queue(500).await.is_ok());
}
