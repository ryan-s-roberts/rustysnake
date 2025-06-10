use ipc_channel::ipc::{IpcSender, IpcReceiver, channel, IpcOneShotServer};
use std::process::Command;
use serde::{Serialize, Deserialize};
use pyo3::prelude::*;
use pyo3::types::{PyModule, PyDict};
use std::fs;
use std::ffi::CString;
use pyo3::py_run;
use tracing::{debug, error};
use rand::{seq::SliceRandom, thread_rng};
use tokio;
use std::thread;
use std::time::Instant;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering, AtomicBool};
use tokio::sync::{mpsc, oneshot};
use ctrlc;
use std::sync::Mutex;
use sysinfo::{System, Process, Pid, get_current_pid, ProcessesToUpdate};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Serialize, Deserialize, Debug)]
struct WeatherRequest {
    city: String,
}

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

fn start_python_worker(worker_id: usize, mut rx: mpsc::UnboundedReceiver<PythonRequest>, shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let locals = pyo3::types::PyDict::new(py);
            let add_path = CString::new("import sys; sys.path.insert(0, '/Users/ryan/code/rustysnake/rustysnake/host')").unwrap();
            py.run(add_path.as_c_str(), Some(&locals), Some(&locals)).unwrap();
            // Inject DummyLM definition into every worker
            let dspy_lm_config = CString::new(r#"
import dspy

class DummyLM(dspy.LM):
    def __init__(self, **kwargs):
        super().__init__("dummy", **kwargs)
        self.model = "dummy"
    def __call__(self, prompt=None, messages=None, signature=None, **kwargs):
        import json
        result = json.dumps({
            "reasoning": "dummy reasoning",
            "next_thought": "dummy next_thought",
            "next_tool_name": "finish",
            "next_tool_args": {},
        })
        return [result]
dspy.settings.configure(lm=DummyLM(), adapter=dspy.JSONAdapter())
"#);
            py.run(dspy_lm_config.expect("CString failed").as_c_str(), Some(&locals), Some(&locals)).unwrap();
            let importlib = py.import("importlib").unwrap();
            let dspy_weather = py.import("dspy_weather").unwrap();
            // Ensure lookup_population is available in dspy_weather module
            let lookup_population_py = pyo3::wrap_pyfunction!(lookup_population, py).unwrap();
            dspy_weather.setattr("lookup_population", lookup_population_py).unwrap();
            importlib.call_method1("reload", (dspy_weather,)).unwrap();
            let dspy_weather = py.import("dspy_weather").unwrap();
            let react = dspy_weather.getattr("react").unwrap();
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let req = match rx.try_recv() {
                    Ok(req) => req,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    },
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                };
                tracing::debug!(worker_id = worker_id, city = %req.question, "Executing request");
                let result = (|| -> Result<String, String> {
                    let kwargs = pyo3::types::PyDict::new(py);
                    kwargs.set_item("question", req.question.clone()).unwrap();
                    // Inject Rust lookup_population thunk into Python context
                    let lookup_population_py = pyo3::wrap_pyfunction!(lookup_population, py).unwrap();
                    locals.set_item("lookup_population", lookup_population_py).unwrap();
                    // Print type and value of result before extracting
                    let py_result = react.call((), Some(&kwargs));
                    match &py_result {
                        Ok(obj) => {
                            let type_name = match obj.get_type().name() {
                                Ok(n) => n.to_str().unwrap_or("unknown").to_owned(),
                                Err(_) => "unknown".to_owned(),
                            };
                            let value_str = match obj.str() {
                                Ok(s) => s.to_str().unwrap_or("").to_owned(),
                                Err(_) => "".to_owned(),
                            };
                            tracing::debug!(worker_id = worker_id, result_type = %type_name, result_str = %value_str, "Python result type and value");
                        }
                        Err(e) => {
                            tracing::error!(worker_id = worker_id, error = %e, "Python error before extraction");
                        }
                    }
                    match py_result {
                        Ok(result) => {
                            // Log the type name
                            let type_name = match result.get_type().name() {
                                Ok(n) => n.to_str().unwrap_or("unknown").to_owned(),
                                Err(_) => "unknown".to_owned(),
                            };
                            // Log the repr
                            let repr = result.repr()
                                .and_then(|r| r.str())
                                .map(|s| s.to_str().unwrap_or("<repr error>").to_owned())
                                .unwrap_or_else(|_| "<repr error>".to_owned());
                            // Check for PyString
                            let is_pystring = result.is_instance_of::<pyo3::types::PyString>();
                            // Check for PyDict
                            let is_pydict = result.is_instance_of::<pyo3::types::PyDict>();
                            // Try extracting as &str
                            let as_str = result.extract::<&str>().unwrap_or("<extract &str error>");
                            // Try extracting as String
                            let as_string = result.extract::<String>().unwrap_or_else(|_| "<extract String error>".to_owned());
                            tracing::debug!(worker_id = worker_id, type_name = %type_name, repr = %repr, is_pystring, is_pydict, as_str = %as_str, as_string = %as_string, "Detailed Python result diagnostics");

                            // If result is a PyString, return the extracted string
                            if is_pystring {
                                return Ok(as_string);
                            }
                            if let Ok(answer_attr) = result.getattr("answer") {
                                if let Ok(answer) = answer_attr.extract::<String>() {
                                    Ok(answer)
                                } else {
                                    Ok(format!("{:?}", answer_attr))
                                }
                            } else if let Ok(dict) = result.downcast::<pyo3::types::PyDict>() {
                                if let Ok(Some(answer_obj)) = dict.get_item("answer") {
                                    Ok(answer_obj.extract::<String>().unwrap_or_else(|_| format!("{:?}", answer_obj)))
                                } else {
                                    Ok(format!("{:?}", dict))
                                }
                            } else {
                                Ok(result.str().unwrap().to_str().unwrap().to_string())
                            }
                        }
                        Err(e) => {
                            tracing::error!(worker_id = worker_id, error = %e, "Python error");
                            Err(format!("Python error: {e}"))
                        },
                    }
                })();
                tracing::debug!(worker_id = worker_id, city = %req.question, ?result, "Sending response for city");
                if let Ok(ref answer) = result {
                    tracing::debug!(worker_id = worker_id, answer = %answer, "Extracted answer string");
                }
                if let Err(e) = req.response_tx.send(result) {
                    tracing::error!(worker_id = worker_id, ?e, "ERROR sending response");
                }
            }
            tracing::debug!(worker_id = worker_id, "Exiting worker loop");
        });
    });
}

#[pyfunction]
fn lookup_weather(city: String) -> PyResult<String> {
    debug!("lookup_weather thunk called for city: {}, thread: {:?}", city, thread::current().id());
    Ok(format!("The weather in {} is always sunny!", city))
}

#[pyo3::pyfunction]
fn lookup_population(city: String) -> i32 {
    tracing::info!("lookup_population thunk 🐍 -> 🦀 called for city: {}", city);
    // Example: return a fake population
    match city.as_str() {
        "Paris" => 2_140_000,
        "London" => 8_900_000,
        "New York" => 8_400_000,
        _ => 1_000_000,
    }
}

fn run_python(question: String) -> String {
    let thread_id = std::thread::current().id();
    let start = std::time::Instant::now();
    tracing::debug!("START question = {}, task thread = {:?}", question, thread_id);
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let repo_root = exe_path
        .parent().and_then(|p| p.parent()).and_then(|p| p.parent())
        .expect("Failed to find repo root");
    let script_path = repo_root.join("rustysnake/host/dspy_weather.py");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|_| panic!("Failed to read {:?}", script_path));
    let script_cstr = std::ffi::CString::new(script).unwrap();
    let answer = Python::with_gil(|py| {
        let lookup_population_py = pyo3::wrap_pyfunction!(lookup_population, py).unwrap();
        let locals = pyo3::types::PyDict::new(py);
        locals.set_item("lookup_population", lookup_population_py).unwrap();
        py.run(script_cstr.as_c_str(), Some(&locals), Some(&locals)).unwrap();
        let react = locals.get_item("react").expect("react missing");
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("question", question.clone()).unwrap();
        let result = react.expect("react missing").call((), Some(&kwargs)).unwrap();
        // Try to extract 'answer' attribute if result is a Prediction object
        if let Ok(answer_attr) = result.getattr("answer") {
            if let Ok(answer) = answer_attr.extract::<String>() {
                answer
            } else {
                format!("{:?}", answer_attr)
            }
        } else if let Ok(dict) = result.downcast::<pyo3::types::PyDict>() {
            if let Ok(Some(answer_obj)) = dict.get_item("answer") {
                answer_obj.extract::<String>().unwrap_or_else(|_| format!("{:?}", answer_obj))
            } else {
                format!("{:?}", dict)
            }
        } else {
            result.str().unwrap().to_str().unwrap().to_string()
        }
    });
    let duration = start.elapsed();
    tracing::debug!("END question = {}, duration = {:?}, task thread = {:?}", question, duration, thread_id);
    tracing::info!("🦀 -> 🐍 -> 🦀 question = {}, answer = {}", question, answer);
    answer
}

#[tokio::main]
async fn main() {
    // Set up tracing to always log at INFO level or lower, output to stdout
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_span_events(FmtSpan::FULL)
        .with_writer(std::io::stdout)
        .init();
    // At the very start, ensure Python env is set before interpreter starts
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let repo_root = exe_path
        .parent().and_then(|p| p.parent()).and_then(|p| p.parent())
        .expect("Failed to find repo root");
    let venv_path = repo_root.join("rustysnake/host/pyenv");
    let site_packages = venv_path.join("lib/python3.13/site-packages");
    let venv_bin = venv_path.join("bin");
    let pythonpath = std::env::var_os("PYTHONPATH");
    let need_reexec = match &pythonpath {
        Some(val) => !val.to_string_lossy().contains(site_packages.to_string_lossy().as_ref()),
        None => true,
    };
    if need_reexec {
        let mut cmd = std::process::Command::new(&exe_path);
        cmd.args(std::env::args().skip(1));
        // Set PYTHONPATH
        cmd.env("PYTHONPATH", &site_packages);
        // Prepend venv/bin to PATH
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let new_path = std::env::join_paths(
            std::iter::once(venv_bin.into_os_string())
                .chain(std::env::split_paths(&old_path).map(|p| p.into_os_string()))
        ).unwrap();
        cmd.env("PATH", &new_path);
        // Preserve DYLD_LIBRARY_PATH if set
        if let Some(dyld) = std::env::var_os("DYLD_LIBRARY_PATH") {
            cmd.env("DYLD_LIBRARY_PATH", dyld);
        }
        // Inherit all other env vars
        cmd.envs(std::env::vars_os());
        let err = cmd.status().expect("Failed to re-exec self");
        std::process::exit(err.code().unwrap_or(1));
    }
    pyo3::prepare_freethreaded_python();
    debug!("Embedding Python with PyO3 and in-process thunks");
    let num_workers = 10;
    let mut worker_senders: Vec<mpsc::UnboundedSender<PythonRequest>> = Vec::new();
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        ctrlc::set_handler(move || {
            shutdown.store(true, Ordering::SeqCst);
            eprintln!("\nReceived shutdown signal. Exiting...");
        }).expect("Error setting Ctrl-C handler");
    }
    for i in 0..num_workers {
        let (tx, rx) = mpsc::unbounded_channel::<PythonRequest>();
        start_python_worker(i, rx, shutdown.clone());
        worker_senders.push(tx);
    }
    let worker_senders = Arc::new(worker_senders);
    let rr_counter = Arc::new(AtomicUsize::new(0));

    let mut cities = vec![
        "London", "Paris", "New York", "Tokyo", "Berlin", "Sydney", "Moscow", "Toronto", "Beijing", "Rio",
        "Madrid", "Rome", "Cairo", "Mumbai", "Seoul", "Bangkok", "Istanbul", "Dubai", "Singapore", "Los Angeles",
        "Chicago", "San Francisco", "Boston", "Dublin", "Vienna", "Prague", "Budapest", "Warsaw", "Brussels", "Amsterdam",
        "Stockholm", "Oslo", "Helsinki", "Copenhagen", "Zurich", "Geneva", "Lisbon", "Barcelona", "Athens", "Edinburgh",
        "Venice", "Florence", "Munich", "Hamburg", "Frankfurt", "Cologne", "Stuttgart", "Dusseldorf", "Leipzig", "Dresden"
    ];
    let original_cities = cities.clone();
    let total_requests = 10;
    while cities.len() < total_requests {
        cities.extend_from_slice(&original_cities);
    }
    cities.truncate(total_requests);
    let worker_counts = Arc::new((0..num_workers).map(|_| Mutex::new(0usize)).collect::<Vec<_>>());
    let start_time = std::time::Instant::now();
    let mut handles = Vec::new();
    for (i, city) in cities.iter().enumerate() {
        let worker_senders = Arc::clone(&worker_senders);
        let rr_counter = Arc::clone(&rr_counter);
        let shutdown = shutdown.clone();
        let city = city.to_string();
        let worker_counts = Arc::clone(&worker_counts);
        let handle = tokio::spawn(async move {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            let idx = rr_counter.fetch_add(1, Ordering::Relaxed) % worker_senders.len();
            // Track which worker handled this request
            {
                let mut count = worker_counts[idx].lock().unwrap();
                *count += 1;
            }
            let tx = &worker_senders[idx];
            let (resp_tx, resp_rx) = oneshot::channel();
            let send_start = std::time::Instant::now();
            if tx.send(PythonRequest { question: city.clone(), response_tx: resp_tx }).is_err() {
                eprintln!("{}: Failed to send to worker", city);
                return;
            }
            let send_duration = send_start.elapsed();
            match resp_rx.await {
                Ok(Ok(_answer)) => {}, // Don't print every answer
                Ok(Err(e)) => eprintln!("{}: Python error: {}", city, e),
                Err(_) => eprintln!("{}: Worker dropped", city),
            }
            // Optionally: track send_duration for backpressure
            if send_duration > std::time::Duration::from_millis(10) {
                println!("[BACKPRESSURE] Request for {} waited {:?} to send to worker {}", city, send_duration, idx);
            }
        });
        handles.push(handle);
    }
    // --- System debug sampling ---
    let (sysdebug_tx, sysdebug_rx) = std_mpsc::channel();
    let sysdebug_shutdown = shutdown.clone();
    std::thread::spawn(move || {
        let mut sys = System::new_all();
        let pid = get_current_pid().unwrap();
        let mut mem_samples = Vec::new();
        let mut thread_samples = Vec::new();
        while !sysdebug_shutdown.load(Ordering::SeqCst) {
            sys.refresh_processes(ProcessesToUpdate::All, true);
            if let Some(proc) = sys.process(pid) {
                mem_samples.push(proc.memory());
                let thread_count = proc.tasks().map(|tasks| tasks.len()).unwrap_or(0);
                thread_samples.push(thread_count as u64);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        let _ = sysdebug_tx.send((mem_samples, thread_samples));
    });
    // Wait for all tasks to finish
    for handle in handles {
        let _ = handle.await;
    }
    let elapsed = start_time.elapsed();
    let total = total_requests;
    let throughput = total as f64 / elapsed.as_secs_f64();
    println!("\n--- Throughput Report ---");
    println!("Total requests: {}", total);
    println!("Elapsed time: {:.2?}", elapsed);
    println!("Throughput: {:.2} requests/sec", throughput);
    println!("Worker utilization:");
    for (i, count) in worker_counts.iter().enumerate() {
        let count = count.lock().unwrap();
        println!("  Worker {}: {} requests ({:.1}%)", i, *count, 100.0 * (*count as f64) / (total as f64));
    }
    // System debug stats
    if let Ok((mem_samples, thread_samples)) = sysdebug_rx.try_recv() {
        if !mem_samples.is_empty() {
            let min_mem = mem_samples.iter().min().unwrap();
            let max_mem = mem_samples.iter().max().unwrap();
            let avg_mem = mem_samples.iter().sum::<u64>() as f64 / mem_samples.len() as f64;
            println!("Memory (resident): min {:.2} MB, max {:.2} MB, avg {:.2} MB", *min_mem as f64 / 1024.0, *max_mem as f64 / 1024.0, avg_mem / 1024.0);
        }
        if !thread_samples.is_empty() {
            let min_threads = thread_samples.iter().min().unwrap();
            let max_threads = thread_samples.iter().max().unwrap();
            let avg_threads = thread_samples.iter().sum::<u64>() as f64 / thread_samples.len() as f64;
            println!("Threads: min {}, max {}, avg {:.1}", min_threads, max_threads, avg_threads);
        }
    }
    println!("------------------------\n");
    Python::with_gil(|py| {
        let sys = py.import("sys").unwrap();
        let exe: String = sys.getattr("executable").unwrap().extract().unwrap();
        let path: Vec<String> = sys.getattr("path").unwrap().extract().unwrap();
        debug!("PYDBG: sys.executable = {}", exe);
        debug!("PYDBG: sys.path = {:?}", path);
    });
} 