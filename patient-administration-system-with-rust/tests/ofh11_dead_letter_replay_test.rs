//! OFH-11: two replays of one dead letter at once send its event once. The replay takes the dead letter with
//! `DELETE … RETURNING`, so the second waits for the first and then finds it gone; read-then-delete let both
//! queue the event again, each under a new id a receiver could not de-duplicate. Needs PostgreSQL:
//! `cargo test -- --ignored`.

mod common;

use patient_administration_system::Error;
use patient_administration_system::db::repositories::outbox::DeadLetterRepository;
use patient_administration_system::db::sql::{Conn, Stmt, Transact};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn two_replays_at_once_send_the_event_once() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let id = Uuid::new_v4();
    let event_type = format!("ofh11-replay-{id}");
    s.db.run(Stmt::new(
        "INSERT INTO outbox_dead_letters (id, original_id, event_type, payload, created_at, retry_count, last_error) \
         VALUES ($1, $2, $3, '{}'::jsonb, now(), 10, 'gave up')",
        [id.into(), Uuid::new_v4().into(), event_type.clone().into()],
    ))
    .await
    .unwrap();

    // The first replay is under way and has not committed.
    let first = s.db.begin_txn().await.unwrap();
    DeadLetterRepository::replay(&first, id).await.unwrap();

    // The second starts while the first holds the dead letter.
    let db = s.db.clone();
    let second = tokio::spawn(async move {
        let txn = db.begin_txn().await.unwrap();
        let result = DeadLetterRepository::replay(&txn, id).await;
        txn.commit().await.unwrap();
        result
    });
    // Give the second time to reach the dead letter before the first lets go of it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    first.commit().await.unwrap();

    let second = second.await.unwrap();
    assert!(matches!(second, Err(Error::NotFound(_))), "{second:?}");
    let queued =
        s.db.count(Stmt::new(
            "SELECT count(*) AS n FROM outbox_events WHERE event_type = $1",
            [event_type.into()],
        ))
        .await
        .unwrap();
    assert_eq!(queued, 1, "the event is queued once");
}
