use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{BufRead, BufReader},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Json, Router, extract::Query, response::Html, routing::get};
use regex::Regex;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::Deserialize;
use tracing::{debug, info};
#[derive(Debug, Clone)]
struct AppState {
    samples: Arc<Mutex<Vec<Sample>>>,
}
#[derive(Debug, Clone, serde::Serialize)]
struct Sample {
    timestamp: i64,
    temperature: Option<f32>,
    humidity: Option<f32>,
    battery_level: Option<f32>,
    voltage: Option<f32>,
    from: u32,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = env::args().collect();
    let report_folder = args.get(1).unwrap();
    info!("scan path for logs: {report_folder}");
    let mut samples = load_from_file(report_folder);
    samples.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    let samples = Arc::new(Mutex::new(samples));
    let state = AppState {
        samples: samples.clone(),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/data", get(data))
        .with_state(state);

    tokio::task::spawn(listen(samples));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:80").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn listen(samples: Arc<Mutex<Vec<Sample>>>) -> () {
    let mut mqttoptions = MqttOptions::new("realtime-listener", "127.0.0.1", 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(30));

    let (client, mut connection) = AsyncClient::new(mqttoptions, 10);
    client.subscribe("#", QoS::AtLeastOnce).await.unwrap();
    info!("subscribed for updates");

    loop {
        if let Ok(Event::Incoming(Packet::Publish(p))) = connection.poll().await {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&p.payload) {
                if let (Some(ts), Some(from)) = (val["timestamp"].as_i64(), val["from"].as_u64()) {
                    debug!("listener val {}", val);
                    let mut entry = Sample {
                        timestamp: ts,
                        from: from as u32,
                        temperature: None,
                        humidity: None,
                        battery_level: None,
                        voltage: None,
                    };

                    entry.temperature = val["payload"]["temperature"].as_f64().map(|v| v as f32);
                    entry.humidity = val["payload"]["relative_humidity"]
                        .as_f64()
                        .map(|v| v as f32);
                    entry.battery_level =
                        val["payload"]["battery_level"].as_f64().map(|v| v as f32);
                    entry.voltage = val["payload"]["voltage"].as_f64().map(|v| v as f32);
                    samples.lock().unwrap().push(entry);
                }
            }
        }
    }
}
#[derive(Deserialize)]
struct DataParams {
    all: Option<bool>,
}
async fn data(
    axum::extract::State(state): axum::extract::State<AppState>,
    Query(params): Query<DataParams>,
) -> Json<Vec<Sample>> {
    if let Ok(samples) = state.samples.lock() {
        let out = if !params.all.is_some() && samples.len() > 2000 {
            samples.last_chunk::<2000>().unwrap().to_vec()
        } else {
            samples.clone()
        };
        Json(out)
    } else {
        Json(vec![])
    }
}

fn load_from_file(path: &str) -> Vec<Sample> {
    let mut samples: HashMap<(u64, i64), Sample> = HashMap::new();
    let re = Regex::new("payload=\"(.*)\"").unwrap();

    for file in fs::read_dir(path).unwrap() {
        let path = file.unwrap().path();
        if path.is_dir() {
            info!("skipping directory: {}", path.display());
            continue;
        }
        let file = File::open(&path).expect("cannot open log file");
        let reader = BufReader::new(file);
        info!("processing file : {}", path.display());

        for line in reader.lines() {
            let Ok(line) = line else {
                continue;
            };
            if let Some(caps) = re.captures(&line) {
                let raw = &caps[1];
                let json_str = raw.replace("\\\"", "\"");

                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let (Some(ts), Some(from)) =
                        (val["timestamp"].as_i64(), val["from"].as_u64())
                    {
                        debug!("val {}", val);

                        let key = (from, ts);
                        let entry = samples.entry(key).or_insert(Sample {
                            timestamp: ts,
                            from: from as u32,
                            temperature: None,
                            humidity: None,
                            battery_level: None,
                            voltage: None,
                        });

                        entry.temperature =
                            val["payload"]["temperature"].as_f64().map(|v| v as f32);
                        entry.humidity = val["payload"]["relative_humidity"]
                            .as_f64()
                            .map(|v| v as f32);
                        entry.battery_level =
                            val["payload"]["battery_level"].as_f64().map(|v| v as f32);
                        entry.voltage = val["payload"]["voltage"].as_f64().map(|v| v as f32);
                    }
                }
            }
        }
    }
    samples.into_values().collect()
}
async fn index() -> Html<&'static str> {
    info!("index");
    Html(
        r#"<!DOCTYPE html>
<html>
<meta charset="utf-8" />
<title>Meshtastic Temperature</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
<script src="https://cdn.jsdelivr.net/npm/chartjs-adapter-date-fns"></script>
</head>
<body>
<h2>Meshtastic Telemetry</h2>
<canvas id="chart"></canvas>

<script>
const ctx = document.getElementById('chart');
const chart = new Chart(ctx, {
type: 'line',
options: {
animation: false,
interaction: { mode: 'index', intersect: false },
scales: {
x: {
  type: 'time',
  time: {
    unit: 'hour',
    tooltipFormat: 'yyyy-MM-dd HH:mm:ss',
    displayFormats: {
      minute: 'HH:mm',
      hour: 'yyyy-MM-dd HH:mm',
      day: 'MMM dd'
    }
  },
  ticks: {
    autoSkip: true,
    maxTicksLimit: 12
  },
  title: {
    display: true,
    text: 'Time'
  }
},
yTemp: {
type: 'linear',
position: 'left',
title: { display: true, text: 'Temperature (°C)' },
min: -5,
max: 35
},
yHum: {
type: 'linear',
position: 'right',
title: { display: true, text: 'Humidity (%)' },
min: 0,
max: 100,
grid: { drawOnChartArea: false }
},
yBat: {
type: 'linear',
position: 'right',
title: { display: true, text: 'Battery (%)' },
min: 0,
max: 100,
grid: { drawOnChartArea: false }
},
yVolt: {
type: 'linear',
position: 'right',
title: { display: true, text: 'Voltage (V)' },
min: 0,
max: 5,
grid: { drawOnChartArea: false }
}
},
grid: { drawOnChartArea: false }
}
});

async function load() {
const res = await fetch('/data');
const data = await res.json();

const byNode = {};
data.forEach(p => {
if (!byNode[p.from]) byNode[p.from] = [];
byNode[p.from].push(p);
});

chart.data.datasets = Object.entries(byNode).flatMap(([node, samples]) => ([
{
label: `Node ${node} Temp (°C)`,
data: samples
.filter(p => p.temperature != null)
.map(p => ({ x: p.timestamp * 1000, y: p.temperature, s: 0 })),
yAxisID: 'yTemp',
tension: 0.2
},
{
label: `Node ${node} Humidity (%)`,
data: samples
.filter(p => p.humidity != null)
.map(p => ({ x: p.timestamp * 1000, y: p.humidity })),
yAxisID: 'yHum',
tension: 0.2
},
{
label: `Node ${node} Battery (%)`,
data: samples
.filter(p => p.battery_level != null)
.map(p => ({ x: p.timestamp * 1000, y: p.battery_level })),
yAxisID: 'yBat',
tension: 0.2
},
{
label: `Node ${node} Voltage (V)`,
data: samples
.filter(p => p.voltage != null)
.map(p => ({ x: p.timestamp * 1000, y: p.voltage })),
yAxisID: 'yVolt',
tension: 0.2
}
]));

chart.update();
}

load();
</script>
</body>
</html>"#,
    )
}
