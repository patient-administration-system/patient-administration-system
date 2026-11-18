# Benchmarks and load tests (PERF-1, NFR-29)

Two kinds of measurement, for two questions.

- **Benchmarks** (Criterion, `patient-administration-system-with-rust/benches/`): how long one piece of code takes, in isolation, so a change that makes it slower shows. They answer "did this commit make parsing, mapping or booking slower?".
- **Load tests** (`pas-load`, `patient-administration-system-with-rust/load/`): how a running server behaves under many callers at once, through every layer a caller meets (network, compression, request id, rate limit, authentication, handler, database). They answer "how many callers can one server take before it slows down, does it stay correct while it does, and does it recover?".

Neither says what a deployment needs. That depends on its demand (clinics, bookings a day, the peak hour, the HL7 feed's rate), which no deployment has yet given. The budgets are a tripwire for regressions until then.

## Running them

| What | Command | Needs |
|---|---|---|
| Pure benchmarks | `just bench` | nothing |
| Database benchmarks | `just bench-db` | `DATABASE_URL` (migrated) |
| Load test, quick | `just load` (the smoke profile) | `DATABASE_URL` |
| Load test, a profile | `just load load`, `just load stress`, `just load spike`, `just load soak 2h` | `DATABASE_URL` |
| Against a server you run | `cargo run --release -p patient-administration-system-load -- --url URL --profile load` | `DATABASE_URL` of that server's database |

`just load` runs `scripts/load-test.sh`: it builds release binaries, starts the server on `LOAD_PORT` (default 8095) with `PAS_RATE_LIMIT_RPM=0` and logging at `warn`, waits for `/api/health`, runs `pas-load`, and stops the server. The server's log is `target/load-server.log` and the report `target/load-report.json`. Criterion writes HTML reports to `target/criterion/`.

**The rate limit must be off on the server under test.** One client sending thousands of requests a minute is what the limit exists to stop; with it on, the numbers measure the limit. A 429 fails the run.

**Rows.** `pas-load` makes its own schedules and slots in the server's database (the API has no route to create them), named `load-<run id>-<purpose>` and at least 600 days ahead, so they never meet real or seeded appointments. They are not deleted: they are the evidence of the run. On a shared development database that is a few thousand rows a run; drop them with `DELETE ... WHERE service_type LIKE 'load-%'` (and their slots and appointments) when they get in the way. Do not run it against a production database.

## pas-load

### Scenarios

| Scenario | Each iteration | Checks |
|---|---|---|
| `read` | One of: health (5%), an appointment (30%), a slot (20%), a day's calendar (20%), the R5 and R4 capability statements (5% each), an HL7 v2 parse (15%, writes nothing) | budgets |
| `cycle` | Book a free slot, read it back for its `ETag`, cancel it with `If-Match` (NFR-27). A cancelled slot is free again, so the 2000 slots are reused | budgets; audit and outbox |
| `contention` | Every worker books a random one of 20 slots at once. A slot already taken answers 409, counted as *contended*, not an error | **each slot goes to exactly one caller**, over HTTP and in the database; audit and outbox |
| `ihe` | IHE ITI Scheduling: `$find` five proposals, `$hold` one, `$book` it. A proposal someone else took answers 404 (the profile's "no longer available") or 409, counted as contended | budgets |
| `mixed` | 80% `read`, 15% `cycle`, 5% `ihe` | budgets |

After every run: **every appointment booked over HTTP has its `book` audit row and its `AppointmentBooked` outbox event** (NFR-1, NFR-2), and **no 429 was seen**.

### Profiles

| Profile | Phases | Checked |
|---|---|---|
| `smoke` | 4 workers, 10 s per scenario | budgets |
| `load` | warm-up 8 workers 10 s, then 32 workers 60 s | budgets on the steady phase |
| `stress` | 8, 16, 32, 64, 128, 256 workers, 20 s each | names the **knee**, the first step over budget, per scenario; a breach there does not fail the run |
| `spike` | 4 workers 20 s, 128 workers 20 s, 4 workers 20 s | budgets before and after (it must recover), errors only during |
| `soak` | warm-up, then 16 workers in one-minute windows for `--duration` (default 30 m) | budgets per window, and **drift**: each operation's p95 in the last window within twice the first's plus 5 ms |

`--workers`, `--duration` and `--rate` change the main phase. Without `--rate` a phase is **closed**: each worker sends its next request when the last one answers, so a slow server gets fewer requests and its queueing is partly hidden. With `--rate N` it is **open**: N requests a second start whatever the server does, at most `--workers` in flight, and one that cannot start is counted as **shed**. Use an open phase to ask "can it take N a second?".

Latency is the time to the last byte of the body, at the client, in milliseconds; percentiles are exact (nearest rank over every sample). Exit status is 0 when every check passed, 1 when a budget or a correctness check failed, 2 when the run could not start.

### Budgets

`patient-administration-system-with-rust/load/budgets.json`: a p95, a p99 and an error rate per operation, with a default. Latency is not judged on fewer than 100 calls in a phase. **They are provisional.** They were set on a developer laptop (Apple silicon, the podman PostgreSQL 18, other work running), with room for a shared CI runner, as a tripwire for a change that makes something several times slower. Change one only with a line in the log below saying why. Before go-live they must be replaced by figures from the expected demand and the production hardware (see "Sizing").

## What the first runs found

**2026-11-15: every booking spent about 95 ms in one query.** The first smoke run showed a successful booking at a median of 277 ms and a p95 of 449 ms with four callers, 12 bookings a second, while a refused booking took 2 ms and a cancel 34 ms. The booking's clash check (`check_owner_free`, CORE-3: is the room or clinician busy in another schedule at the same time?) asks whether an appointment holds a slot, and `appointments.slot_id` had no index, so PostgreSQL hashed every live appointment (78,000 on the development database) on each booking, and read every schedule twice for want of an index on the owner. Migration 64 adds `appointments (slot_id)`, `schedules (owner_kind, owner_id)` and `slots (schedule_id, start_datetime)`: the query went from 99 ms to 0.04 ms.

| Four callers, smoke profile | Before migration 64 | After |
|---|---|---|
| Book a slot, p50 / p95 | 277 / 449 ms | 6.7 / 8.7 ms |
| Bookings a second (book, read, cancel) | 12 | 348 |
| IHE `$find`, p50 / a second | 111 ms / 9 | 9 ms / 200 |

The cost grows with the appointments table, so a deployment with a year of appointments would have been slower still. No functional test could have shown it: they run on small tables.

**2026-11-15: the database pool was fixed at 10 connections.** With 32 callers, bookings levelled off however many callers were added (stress: 204 a second at 8 callers, 146 at 128), and a cancel waited as long as a booking, which is the sign of callers queueing for a connection. The pool size was hard-coded; it is now `PAS_DB_MAX_CONNECTIONS` (default 10, checked at start-up, `operations/deployment.md`).

| `cycle` under the `load` profile, 32 callers, slots over 50 clinicians | Pool 10 | Pool 40 |
|---|---|---|
| Book, read, cancel cycles a second | 390 | 545 |
| Book a slot, p50 / p95 | 29 / 37 ms | 53 / 93 ms |
| Cancel, p50 / p95 | 36 / 42 ms | 5.6 / 7.2 ms |

With 40 connections the cancels stop queueing and the bookings take the time instead: they are now limited by their own work in the database (the row lock on the slot, the owner lock, the clash check, the audit row and the outbox event in one transaction), not by waiting for a connection. Each replica has its own pool, so replicas times `PAS_DB_MAX_CONNECTIONS` must stay under PostgreSQL's `max_connections` (100 by default).

**By design: bookings for one clinician, room or piece of equipment are one at a time.** A booking takes an advisory lock on its schedule's owner for its transaction (CORE-3), so two bookings arriving together through two schedules of the same clinician cannot both pass the clash check. On this laptop that caps one owner at about 340 bookings a second, far above any one clinician's real rate, and it is why the `cycle` fixture spreads its slots over 50 clinicians: the first version put them all under one and measured that lock.

## Benchmark baselines

From one quick pass (`cargo bench -- --quick`) on the same laptop, 2026-11-15, to read a later run against. Criterion's own report (`target/criterion/report/index.html`) compares each run with the last one on that machine.

| Benchmark | Time |
|---|---|
| HL7 v2 parse, one ADT A01 | 2.8 µs |
| HL7 v2 PID to a person | 1.1 µs |
| NHS number check | 0.17 µs |
| FHIR R5 Appointment mapping | 1.1 µs |
| FHIR R5 searchset of 500 appointments, mapped and serialised | 2.0 ms |
| No-show model, 10,000 appointments | 2.8 ms |
| Book a slot (in process, PostgreSQL) | 3.4 ms |
| Book then cancel (in process) | 6.3 ms |
| Free slots of one schedule, 1,000 slots | 1.2 ms |
| One day's calendar, 2,432 appointments (development database) | 15 ms, of which 1.7 ms in PostgreSQL |
| `GET /api/appointments/{id}` through the router, in process | 0.37 ms |
| One day's calendar through the router, in process | 19.5 ms |

The calendar is the slow one, and it is not paged: see PERF-2 in `tasks.md`.

### Added 2026-11-18 (PERF-3)

Short runs (`--warm-up-time 1 --measurement-time 2`) on the same laptop, against the development database. The weekly Performance job writes the same table for its own run to the run page (`scripts/criterion-summary.py`).

| Benchmark | Median |
|---|---|
| HL7 v2 ACK to an A01 | 3.1 µs |
| HL7 v2 batch ACK, 1,000 messages | 4.5 µs |
| HL7 v2 de-duplication key | 0.63 µs |
| Redact a 1,000-segment message for a log | 124 µs (690 MB/s) |
| Read one MLLP frame, 85 KB | 275 µs (310 MB/s) |
| MLLP round trip, kept open, plain / TLS | 30 µs / 42 µs |
| MLLP round trip, new connection each message, plain / TLS | 72 µs / 207 µs |
| Webhook signature (HMAC-SHA256), 256 B / 64 KB | 0.74 µs / 29 µs |
| Classify a webhook answer: accepted / rejected / deferred with `Retry-After` | 0.35 ns / 57 ns / 63 ns |
| Outbox: the next 100 due, over a backlog of 1,000 / 10,000 / 100,000 | 0.76 ms / 0.76 ms / 0.78 ms |
| HL7 v2 de-duplication in PostgreSQL, new / resent message | 1.3 ms / 1.3 ms |
| Working days: add 1 / 10 / 250 across a year of holidays | 8 ns / 97 ns / 2.4 µs |
| A work item's due date (10 working days) | 72 ns |
| Work queue sync when nothing changed, 1,000 / 10,000 open calls on top of the database | 24.5 ms / 39.6 ms |
| Work queue: take the next item and release it | 2.5 ms |

**Found by these benchmarks.** The work queue's sync (TSC-1) first sent one statement per open item every minute, even when nothing had changed: 0.8 s with 1,000 open calls and 2.1 s with 10,000. It now compares in memory, writes only what changed, and reads no closed history: 24.5 ms and 39.6 ms. The outbox's next batch stays flat as the backlog grows a hundredfold, so its index holds.

## Results log

Each line is one run, on the machine named. Numbers from a busy laptop are evidence of shape and regressions, not capacity.

| Date | Machine | Profile | Result |
|---|---|---|---|
| 2026-11-15 | Apple silicon laptop, podman PostgreSQL 18, load average about 20 from other work | smoke, before migration 64 | FAILED: booking p95 449 ms (budget 200); see above |
| 2026-11-15 | same | smoke, after | PASSED; all checks; 5,813 bookings, each with its audit row and outbox event; 20 contended slots, each to one caller |
| 2026-11-15 | same, load average about 30 | load | PASSED; reads about 2,900 a second (`get_appointment` p95 5 ms), book-read-cancel about 305 cycles a second (book p95 70 ms), 38,118 bookings all audited |
| 2026-11-15 | same | stress (read, cycle, mixed) | knees: cycle at 32 callers, mixed at 128, read at 256 (p95 80 ms, p99 136 ms); no errors at any step |
| 2026-11-15 | same, load average about 8 | load, cycle over 50 clinicians, pool 10 and 40 | PASSED both; 390 and 545 cycles a second (table above) |

## Sizing a deployment

What a deployment should do before go-live, once it knows its demand:

1. Write down the peak: requests a second by kind (staff reads, bookings, the HL7 feed, the patient portal), from the current system's logs or the clinic count.
2. Run `pas-load --profile load --rate <peak × 2>` on the production hardware, against a copy of the database at a year's size, and `--profile soak --duration 8h` at the expected rate.
3. Set `budgets.json` from what the service needs (a booking clerk waits on `book_slot`; an HL7 sender has its own timeout) and record the change here.
4. Run `--profile stress` to find the knee, and keep the peak well below it.

## Not covered

- More than one replica. The jobs are safe with several (NFR-19), but no run has put two servers behind one database.
- The upstream proxy routes (`/api/patients/...`, `/api/practitioners/...`), which call person-service and worker-service; the UI server; the MLLP listener; the background jobs (reminders, the outbox dispatcher), which run during a load test as in production but which no scenario measures; authentication with per-user tokens (`--token` sends one, but the default run is in trusted-caller mode).
- A database at production size. The development database had 78,000 appointments; a year at one hospital is more.
