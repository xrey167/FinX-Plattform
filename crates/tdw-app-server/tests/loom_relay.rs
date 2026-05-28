//! Loom concurrency model for the in-memory outbox -> bus relay
//! (`tdw_app_server::spawn_inmemory_relay`).
//!
//! TEST-POLICY-003 (see `docs/adr/0014-test-policy-backlog.md` and
//! `docs/quality/test-policy-backlog.md`).
//!
//! ## Why this target
//!
//! ADR 0014 originally named `tdw-bus` as the first loom target, but the
//! in-memory `EventBus` is single-threaded by construction: every mutator takes
//! `&mut self`, there is no interior mutability and no atomics, so a loom model
//! would be tautological. The relay in `tdw-app-server` is the smallest
//! genuinely concurrent in-process component in the workspace: it touches two
//! distinct `Mutex`-guarded resources (the outbox and the bus) and performs a
//! check-then-act across THREE separate lock acquisitions per record:
//!
//! ```text
//!   1. lock(outbox)  -> pending_after(last_seq)   // read which records to ship
//!   2. lock(bus)     -> publish(envelope)         // ship to the bus
//!   3. lock(outbox)  -> mark_dispatched(seq)      // record that we shipped it
//! ```
//!
//! Because steps 1 and 3 release the outbox lock between them, a concurrent
//! producer that `append`s a new record can interleave at any of those release
//! points. This is exactly the interleaving-sensitive shape loom exists to
//! check.
//!
//! ## Invariant proven
//!
//! With ONE producer thread appending a record and ONE relay thread running a
//! single drain cycle, for every record that the relay observed as pending:
//!
//!   * it is published to the bus exactly once (no double-publish), and
//!   * it is marked `Dispatched` in the outbox (no lost update where a record
//!     stays `Pending` forever after the relay shipped it).
//!
//! And globally: the bus never contains more entries than were appended (the
//! relay cannot fabricate or duplicate events), and the number of dispatched
//! records equals the number of bus entries (outbox and bus stay consistent).
//!
//! ## How to run
//!
//! This test only compiles under `--cfg loom`; the default build and default
//! `cargo test` never see it.
//!
//! ```powershell
//! $env:RUSTFLAGS = "--cfg loom"
//! cargo test -p tdw-app-server --test loom_relay --target-dir C:\Users\ReyDa\loom-target
//! Remove-Item Env:\RUSTFLAGS
//! ```
//!
//! If loom reports "the number of branches is too large", lower the work with
//! `$env:LOOM_MAX_PREEMPTIONS = "2"` before running.

#![cfg(loom)]

use loom::sync::{Arc, Mutex};
use loom::thread;

/// Status of a record, mirroring `tdw_outbox::OutboxStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Pending,
    Dispatched,
}

/// A single record, mirroring `tdw_outbox::OutboxRecord` (sequence + status;
/// the payload is irrelevant to the locking invariant so it is omitted).
#[derive(Clone, Copy)]
struct Record {
    sequence: u64,
    status: Status,
}

/// Minimal faithful extraction of `tdw_outbox::InMemoryOutbox`'s locking-
/// relevant logic: `append`, `pending_after`, and `mark_dispatched` keep the
/// same semantics as the real type.
#[derive(Default)]
struct Outbox {
    next_sequence: u64,
    records: Vec<Record>,
}

impl Outbox {
    fn append(&mut self) -> u64 {
        let sequence = if self.next_sequence == 0 {
            1
        } else {
            self.next_sequence
        };
        self.next_sequence = sequence + 1;
        self.records.push(Record {
            sequence,
            status: Status::Pending,
        });
        sequence
    }

    fn pending_after(&self, sequence: u64) -> Vec<Record> {
        self.records
            .iter()
            .filter(|r| r.sequence > sequence && r.status == Status::Pending)
            .copied()
            .collect()
    }

    fn mark_dispatched(&mut self, sequence: u64) -> bool {
        for r in &mut self.records {
            if r.sequence == sequence {
                r.status = Status::Dispatched;
                return true;
            }
        }
        false
    }

    fn dispatched_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| r.status == Status::Dispatched)
            .count()
    }

    fn pending_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| r.status == Status::Pending)
            .count()
    }
}

/// Minimal faithful extraction of `tdw_bus::EventBus::publish`: an append-only
/// log of published sequences (capacity eviction is irrelevant to the
/// no-loss / no-duplicate invariant for the bounded budget here).
#[derive(Default)]
struct Bus {
    published: Vec<u64>,
}

impl Bus {
    fn publish(&mut self, sequence: u64) {
        self.published.push(sequence);
    }
}

/// One faithful relay drain cycle, mirroring the body of
/// `spawn_inmemory_relay`: read pending under the outbox lock, then for each
/// record publish under the bus lock and mark dispatched under the outbox lock
/// (three separate lock scopes).
fn relay_drain_once(outbox: &Arc<Mutex<Outbox>>, bus: &Arc<Mutex<Bus>>, last_seq: u64) {
    let pending = {
        let guard = outbox.lock().unwrap();
        guard.pending_after(last_seq)
    };
    for record in &pending {
        {
            let mut bus_guard = bus.lock().unwrap();
            bus_guard.publish(record.sequence);
        }
        {
            let mut outbox_guard = outbox.lock().unwrap();
            outbox_guard.mark_dispatched(record.sequence);
        }
    }
}

/// Model: a relay drain cycle races a single concurrent producer append.
///
/// Two threads, bounded permutation budget. The relay starts from `last_seq=0`
/// (it has shipped nothing yet) and there is one pre-existing record (seq 1).
/// Concurrently the producer appends a second record (seq 2). Whatever the
/// interleaving, the model asserts:
///
///   * the bus never has more entries than records that exist (no fabrication),
///   * the bus has no duplicate sequences (no double-publish),
///   * every record the relay published is marked Dispatched (no lost update),
///   * dispatched_count == bus.published.len() (outbox/bus stay consistent).
#[test]
fn relay_and_producer_no_lost_update_no_double_publish() {
    loom::model(|| {
        let outbox = Arc::new(Mutex::new(Outbox::default()));
        let bus = Arc::new(Mutex::new(Bus::default()));

        // Seed one pending record (sequence 1) before the race begins.
        {
            let mut guard = outbox.lock().unwrap();
            let seq = guard.append();
            assert_eq!(seq, 1);
        }

        let relay_outbox = Arc::clone(&outbox);
        let relay_bus = Arc::clone(&bus);
        let relay = thread::spawn(move || {
            relay_drain_once(&relay_outbox, &relay_bus, 0);
        });

        let producer_outbox = Arc::clone(&outbox);
        let producer = thread::spawn(move || {
            let mut guard = producer_outbox.lock().unwrap();
            let _seq = guard.append();
        });

        relay.join().unwrap();
        producer.join().unwrap();

        let outbox_guard = outbox.lock().unwrap();
        let bus_guard = bus.lock().unwrap();

        // Exactly two records exist (seq 1 seeded, seq 2 appended by producer).
        assert_eq!(
            outbox_guard.records.len(),
            2,
            "exactly two records should exist"
        );

        // No fabrication: the bus cannot hold more events than records exist.
        assert!(
            bus_guard.published.len() <= outbox_guard.records.len(),
            "bus must not fabricate events"
        );

        // No double-publish: published sequences are unique.
        let mut seen = bus_guard.published.clone();
        seen.sort_unstable();
        let unique_len = {
            let mut s = seen.clone();
            s.dedup();
            s.len()
        };
        assert_eq!(
            seen.len(),
            unique_len,
            "bus must not contain duplicate sequences (no double-publish)"
        );

        // Consistency: every published record is marked Dispatched, and the
        // dispatched count equals the number of bus entries (no lost update).
        assert_eq!(
            outbox_guard.dispatched_count(),
            bus_guard.published.len(),
            "every published record must be marked dispatched"
        );

        // Whatever the relay shipped, it shipped fully: a record is never left
        // Pending after appearing on the bus.
        for &seq in &bus_guard.published {
            let record = outbox_guard
                .records
                .iter()
                .find(|r| r.sequence == seq)
                .expect("published sequence must correspond to a real record");
            assert_eq!(
                record.status,
                Status::Dispatched,
                "published record {seq} must be Dispatched, not left Pending"
            );
        }

        // Total accounting holds: dispatched + pending == total records.
        assert_eq!(
            outbox_guard.dispatched_count() + outbox_guard.pending_count(),
            outbox_guard.records.len(),
            "every record is either Pending or Dispatched"
        );
    });
}
