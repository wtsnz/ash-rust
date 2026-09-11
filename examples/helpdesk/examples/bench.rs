use std::time::{Duration, Instant};
use ash_memory::Memory;
use helpdesk::ticket::fields as t;
use helpdesk::{Helpdesk, Status, Ticket, actor_customer};
use uuid::Uuid;

async fn run_bench<F, Fut>(name: &str, warmup_dur: Duration, bench_dur: Duration, mut op: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    print!("Warming up {name} ({}s)...", warmup_dur.as_secs());
    let warmup_start = Instant::now();
    let mut warmup_count = 0u64;
    while warmup_start.elapsed() < warmup_dur {
        op().await;
        warmup_count += 1;
    }
    println!(" done ({warmup_count} iterations).");

    print!("Benchmarking {name} ({}s)...", bench_dur.as_secs());
    let mut latencies = Vec::with_capacity(1_000_000);
    let start = Instant::now();
    while start.elapsed() < bench_dur {
        let op_start = Instant::now();
        op().await;
        latencies.push(op_start.elapsed());
    }
    let total_elapsed = start.elapsed();
    let count = latencies.len() as f64;
    let ips = count / total_elapsed.as_secs_f64();

    latencies.sort_unstable();
    let avg = total_elapsed / latencies.len() as u32;
    let median = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

    println!(" completed!\n");
    println!("--------------------------------------------------");
    println!("Benchmark: {name}");
    println!("  Iterations:   {:>10}", latencies.len());
    println!("  Throughput:   {:>10.2} ips", ips);
    println!("  Average:      {:>10.2} µs", avg.as_nanos() as f64 / 1000.0);
    println!("  Median:       {:>10.2} µs", median.as_nanos() as f64 / 1000.0);
    println!("  P99:          {:>10.2} µs", p99.as_nanos() as f64 / 1000.0);
    println!("--------------------------------------------------\n");
}

#[tokio::main]
async fn main() {
    println!("\n=== Starting ash-rust Benchmark ===");

    // Benchmark 1: Ticket.open
    {
        let desk = Helpdesk::new(Memory::new());
        let customer = desk.with_actor(actor_customer(Uuid::new_v4()));
        run_bench(
            "Ticket.open (validate + changeset + in-memory)",
            Duration::from_secs(2),
            Duration::from_secs(5),
            || async {
                customer.open_ticket("Printer is broken").await.unwrap();
            },
        )
        .await;
    }

    // Benchmark 2: Representative.create
    {
        let desk = Helpdesk::new(Memory::new());
        run_bench(
            "Representative.create (validate + in-memory)",
            Duration::from_secs(2),
            Duration::from_secs(5),
            || async {
                desk.create_representative("Alice Smith").await.unwrap();
            },
        )
        .await;
    }

    // Benchmark 3: Ticket.read (filter status == open, 100+ records)
    {
        let desk = Helpdesk::new(Memory::new());
        let customer = desk.with_actor(actor_customer(Uuid::new_v4()));
        for i in 1..=100 {
            customer.open_ticket(&format!("Ticket number {i}")).await.unwrap();
        }

        run_bench(
            "Ticket.read (filter status == open, 100+ records)",
            Duration::from_secs(2),
            Duration::from_secs(5),
            || async {
                let _ = Ticket::query(&customer)
                    .filter(t::status.eq(Status::Open))
                    .all()
                    .await
                    .unwrap();
            },
        )
        .await;
    }
}
