use chrono::Local;
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use std::{env, fs::OpenOptions, io::Write, time::Duration};

fn main() {
    let args: Vec<String> = env::args().collect();
    let ts = Local::now().format("%Y%m%d%H%M%S");
    let file = format!("{}_{}", args.get(1).unwrap(), ts);
    let mut mqttoptions = MqttOptions::new("file-logger", "127.0.0.1", 1883);
    println!("writing logs to {}", file);
    mqttoptions.set_keep_alive(Duration::from_secs(30));

    let (client, mut connection) = Client::new(mqttoptions, 10);
    client.subscribe("#", QoS::AtLeastOnce).unwrap();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .unwrap();

    for notification in connection.iter() {
        if let Ok(Event::Incoming(Packet::Publish(p))) = notification {
            writeln!(
                file,
                "topic={} payload={:?}",
                p.topic,
                String::from_utf8_lossy(&p.payload)
            )
            .unwrap();
        }
    }
}
