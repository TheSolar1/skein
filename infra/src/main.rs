use skein::{gateway, identity::{fmt_addr, Identity}, meshtastic::Meshtastic, node::Node, transport::{Transport, Udp}};
use std::{path::PathBuf, sync::Arc};

const USAGE: &str = "skein run  [--listen 0.0.0.0:7777] [--peer host:port]... [--site DIR --name NAME] [--mesh radio-host:4403 | --serial COM5] [--gateway 127.0.0.1:8080] [--key skein.key]\nskein addr [--key skein.key]";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let (mut listen, mut gw, mut key) = ("0.0.0.0:7777".to_string(), "127.0.0.1:8080".to_string(), PathBuf::from("skein.key"));
    let (mut peers, mut site, mut name, mut mesh) = (vec![], None::<PathBuf>, None::<String>, None::<String>);
    let mut serial = None::<String>;
    let mut no_discover = false;
    while let Some(a) = args.next() {
        if a == "--no-discover" {
            no_discover = true;
            continue;
        }
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--listen" => listen = v,
            "--peer" => peers.push(v),
            "--site" => site = Some(v.into()),
            "--name" => name = Some(v),
            "--mesh" => mesh = Some(v),
            "--serial" => serial = Some(v),
            "--gateway" => gw = v,
            "--key" => key = v.into(),
            _ => return eprintln!("{USAGE}"),
        }
    }
    let id = Identity::load_or_create(&key).expect("cannot read or create the key file");
    match cmd.as_str() {
        "addr" => println!("{}", fmt_addr(&id.addr())),
        "run" => {
            let udp = Udp::bind(listen.parse().expect("bad --listen"), peers).await.expect("cannot bind UDP port");
            let udp = Arc::new(udp);
            if !no_discover {
                udp.discover();
                println!("looking for other nodes on this network (no internet needed)");
            }
            let mut transports = vec![udp as Arc<dyn Transport>];
            if let Some(addr) = mesh {
                transports.push(Arc::new(Meshtastic::connect(&addr).await.expect("cannot reach the Meshtastic radio")));
                println!("meshtastic radio {addr}");
            }
            if let Some(port) = serial {
                transports.push(Arc::new(Meshtastic::connect_serial(&port).await.expect("cannot open the serial port")));
                println!("meshtastic radio on {port}");
            }
            let node = Node::new(id, transports);
            println!("address {}", fmt_addr(&node.id.addr()));
            node.start();
            if let (Some(dir), Some(name)) = (site, name) {
                match node.publish(dir, &name).await {
                    Ok(()) => println!("publishing as http://{name}.skein.localhost:{}/", gw.rsplit(':').next().unwrap_or("8080")),
                    Err(e) => return eprintln!("{e}"),
                }
            }
            println!("gateway http://{gw}/ (open a .skein name as http://NAME.skein.localhost:PORT/)");
            gateway::serve(node, gw.parse().expect("bad --gateway")).await.expect("gateway stopped");
        }
        _ => eprintln!("{USAGE}"),
    }
}
