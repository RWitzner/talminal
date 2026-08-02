use base64::Engine as _;
use serde::Serialize;
use std::hint::black_box;
use std::time::Instant;

#[derive(Serialize)]
struct PtyOutputEvent {
    name: String,
    data_b64: String,
}

/// Reproducerbar mikrobenchmark for §5's Rust-del.
///
/// Ignoreret i ritualet, fordi testen maaler tid og ikke korrekthed:
/// `cargo test --release --test perf_pty_output -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_base64_plus_json_per_chunk() {
    const CHUNKS: usize = 250_000;
    const BYTES_PER_CHUNK: usize = 4_096;
    let bytes: Vec<u8> = (0..BYTES_PER_CHUNK).map(|i| (i % 251) as u8).collect();

    for _ in 0..10_000 {
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        black_box(
            serde_json::to_string(&PtyOutputEvent {
                name: "card-source".into(),
                data_b64,
            })
            .unwrap(),
        );
    }

    let started = Instant::now();
    let mut encoded_bytes = 0usize;
    for _ in 0..CHUNKS {
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        encoded_bytes += data_b64.len();
        black_box(
            serde_json::to_string(&PtyOutputEvent {
                name: "card-source".into(),
                data_b64,
            })
            .unwrap(),
        );
    }
    let elapsed = started.elapsed();
    let us_per_chunk = elapsed.as_secs_f64() * 1_000_000.0 / CHUNKS as f64;
    println!(
        "{{\"chunks\":{CHUNKS},\"bytes_per_chunk\":{BYTES_PER_CHUNK},\
         \"base64_bytes\":{encoded_bytes},\"total_ms\":{:.3},\
         \"base64_plus_json_us_per_chunk\":{us_per_chunk:.4}}}",
        elapsed.as_secs_f64() * 1_000.0,
    );
}
