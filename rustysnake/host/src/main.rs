use ipc_channel::ipc::{IpcSender, IpcReceiver, channel, IpcOneShotServer};
use std::process::Command;
use serde::{Serialize, Deserialize};
use pyo3::prelude::*;
use pyo3::types::{PyModule, PyDict, PyInt};
use std::fs;
use std::ffi::CString;
use pyo3::py_run;
use tracing::{debug, error};
use rand::{seq::SliceRandom, thread_rng};
use pyo3_async_runtimes::tokio::main;
use tokio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering, AtomicBool};
use tokio::sync::{mpsc, oneshot};
use ctrlc;
use std::sync::Mutex;
use sysinfo::{System, Process, Pid, get_current_pid, ProcessesToUpdate};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tracing_subscriber::fmt::format::FmtSpan;
use futures::future::join_all;
use pyo3_async_runtimes::tokio::future_into_py;
use std::ffi::CStr;
use libc;
use pyo3::IntoPyObjectExt;
use pyo3::Py;
use std::clone::Clone;

#[derive(Serialize, Deserialize, Debug)]
struct ThunkRequest {
    city: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkerChannels {
    city_rx: IpcReceiver<String>,
    result_tx: IpcSender<String>,
    thunk_tx: IpcSender<ThunkRequest>,
    thunk_reply_rx: IpcReceiver<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Handshake {
    worker_tx: IpcSender<WorkerChannels>,
}

struct PythonRequest {
    question: String,
    response_tx: oneshot::Sender<Result<String, String>>,
}

static LOOKUP_POPULATION_CALLS: AtomicUsize = AtomicUsize::new(0);

#[pyfunction]
fn lookup_population(py: Python, city: String) -> PyResult<Bound<PyAny>> {
    println!("lookup_population called");
    LOOKUP_POPULATION_CALLS.fetch_add(1, Ordering::Relaxed);
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        // Simulate async work, e.g., tokio::time::sleep
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // Return a dummy population for demonstration
        let pop = if city == "Paris" { 2_140_526 } else { 0 };
        Ok(pop)
    })
}

#[main]
async fn main() -> PyResult<()> {
    use std::time::Instant;
    println!("Starting host (async, single-threaded)");
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_span_events(FmtSpan::FULL)
        .with_writer(std::io::stdout)
        .init();

    // Set up Python environment and inject Rust thunk
    Python::with_gil(|py| {
        let sys = py.import("sys").unwrap();
        sys.getattr("path").unwrap().call_method1("insert", (0, "/Users/ryan/code/rustysnake/rustysnake/host")).unwrap();
        py.import("dummy_lm").unwrap();
        let dspy_weather = py.import("dspy_weather").unwrap();
        let lookup_population_py = wrap_pyfunction!(lookup_population, py).unwrap();
        dspy_weather.setattr("lookup_population", lookup_population_py).unwrap();
    });

    // Call lookup_population directly and via react_async
    let (direct_coro, react_coro) = Python::with_gil(|py| {
        let dspy_weather = py.import("dspy_weather").unwrap();
        let lookup_population = dspy_weather.getattr("lookup_population").unwrap();
        let city_arg = "Paris";
        let direct_coro = lookup_population.call1((city_arg,)).unwrap().unbind();

        let react_async = dspy_weather.getattr("react_async").unwrap();
        let kwargs = PyDict::new(py);
        kwargs.set_item("question", "Paris").unwrap();
        let react_coro = react_async.call((), Some(&kwargs)).unwrap().unbind();
        (direct_coro, react_coro)
    });

    // Await both coroutines from Rust
    let direct_result = Python::with_gil(|py| {
        pyo3_async_runtimes::tokio::into_future(direct_coro.bind(py).clone())
    })?.await?;
    println!("Direct lookup_population result: {:?}", direct_result);

    let react_result = Python::with_gil(|py| {
        pyo3_async_runtimes::tokio::into_future(react_coro.bind(py).clone())
    })?.await?;
    println!("react_async result: {:?}", react_result);

    // Make 1000 concurrent async calls to DSPy (Python), tracing start and completion
    let questions: Vec<_> = (0..10).map(|i| (i, format!("city_{}", i))).collect();
    let start = Instant::now();
    let futures = questions.into_iter().map(|(idx, city)| async move {
        tracing::info!(index = idx, city = %city, "Starting DSPy call");
        let answer_py_fut = Python::with_gil(|py| {
            let sys = py.import("sys").unwrap();
            sys.getattr("path").unwrap().call_method1("insert", (0, "/Users/ryan/code/rustysnake/rustysnake/host")).unwrap();
            let dspy_weather = py.import("dspy_weather").unwrap();
            let react_async = dspy_weather.getattr("react_async").unwrap();
            let kwargs = PyDict::new(py);
            kwargs.set_item("question", &city).unwrap();
            let py_coro = react_async.call((), Some(&kwargs)).unwrap();
            pyo3_async_runtimes::tokio::into_future(py_coro)
        });
        let answer_py = answer_py_fut.unwrap().await.unwrap();
        let _answer: String = Python::with_gil(|py| {
            if let Ok(answer) = answer_py.getattr(py, "answer") {
                answer.extract(py)
            } else if let Ok(to_json) = answer_py.getattr(py, "to_json") {
                let json_str = to_json.call0(py).unwrap();
                json_str.extract(py)
            } else {
                answer_py.bind(py).str().and_then(|s| s.extract())
            }
        }).unwrap();
        tracing::info!(index = idx, city = %city, "Completed DSPy call");
        // No per-response println
    });
    futures::future::join_all(futures).await;
    let elapsed = start.elapsed();
    let total_calls = LOOKUP_POPULATION_CALLS.load(Ordering::Relaxed);
    let throughput = total_calls as f64 / elapsed.as_secs_f64();
    println!("Total lookup_population calls: {}", total_calls);
    println!("Total elapsed: {:.3} seconds", elapsed.as_secs_f64());
    println!("Throughput: {:.2} calls/sec", throughput);
    Ok(())
} 