use core::time;
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::Command,
    sync::{Arc, Condvar, Mutex, RwLock},
    thread::{self, sleep},
};

use clap::Parser;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Opts {
    #[arg(short, long)]
    namespace: Vec<String>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq)]
struct CursorPos {
    x: f32,
    y: f32,
}

#[derive(Deserialize, Debug, Clone)]
struct Layer {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    namespace: String,
    #[serde(skip_deserializing)]
    visible: bool,
}

impl Layer {
    fn does_contain_cursor(&self, cursorpos: &CursorPos) -> bool {
        let y_buffer = self.h * 2.0 / 3.0;
        let mut bar_y_max = self.y + self.h;
        let mut bar_y_min = self.y;

        if self.y > self.h {
            if self.visible {
                bar_y_min -= y_buffer;
            } else {
                bar_y_min += y_buffer;
            };
        } else if self.visible {
            bar_y_max += y_buffer;
        } else {
            bar_y_max -= y_buffer;
        }

        let bar_x_max = self.x + self.w;
        let bar_x_min = self.x;

        cursorpos.y <= bar_y_max
            && cursorpos.y >= bar_y_min
            && cursorpos.x <= bar_x_max
            && cursorpos.x >= bar_x_min
    }

    fn toggle_visibility(&mut self, cursorpos: &CursorPos) -> anyhow::Result<()> {
        let cursor_over_layer = self.does_contain_cursor(cursorpos);
        let toggle = || -> anyhow::Result<()> {
            Command::new("pkill")
                .args(["-SIGUSR1", &self.namespace])
                .spawn()?;
            Ok(())
        };
        if cursor_over_layer && !self.visible {
            toggle()?;
            self.visible = true;
            println!("{} revealed.", self.namespace);
        } else if !cursor_over_layer && self.visible {
            thread::sleep(time::Duration::from_secs(1));
            toggle()?;
            self.visible = false;
            println!("{} hidden.", self.namespace);
        }

        Ok(())
    }
}

type Level = u16;
type Monitor = String;

#[derive(Deserialize, Debug, Clone)]
struct LayerByLevel {
    levels: HashMap<Level, Vec<Layer>>,
}

fn get_layers(namespaces: &Vec<String>, socket: &str) -> anyhow::Result<Vec<Layer>> {
    let mut stream = UnixStream::connect(socket).unwrap();
    let _ = stream.write(b"j/layers");
    let mut layers_str = String::new();
    stream.read_to_string(&mut layers_str).unwrap();
    let levels_by_monitor: HashMap<Monitor, LayerByLevel> = serde_json::from_str(&layers_str)?;

    Ok(levels_by_monitor
        .into_iter()
        .flat_map(|(_, layer_by_level)| {
            layer_by_level
                .levels
                .into_iter()
                .flat_map(|(_, layer)| layer)
                .collect::<Vec<Layer>>()
        })
        .filter_map(|layer| {
            for namespace in namespaces {
                if *namespace == layer.namespace {
                    return Some(Layer {
                        namespace: namespace.to_string(),
                        visible: true,
                        ..layer
                    });
                }
            }
            None
        })
        .collect::<Vec<Layer>>())
}

fn get_cursor_pos(socket: &str) -> anyhow::Result<CursorPos> {
    let mut stream = UnixStream::connect(socket).unwrap();
    let _ = stream.write(b"j/cursorpos");
    let mut cursorpos_str = String::new();
    while cursorpos_str.is_empty() {
        let _ = stream.read_to_string(&mut cursorpos_str);
    }
    Ok(serde_json::from_str(cursorpos_str.as_str())?)
}

#[derive(Deserialize, Debug, Clone)]
struct Client {
    fullscreen: bool,
    floating: bool,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: u16,
}

fn fullscreen_or_floating_focused(socket: &str) -> anyhow::Result<bool> {
    let mut stream = UnixStream::connect(socket).unwrap();
    let _ = stream.write(b"j/clients");
    let mut clients_str = String::new();
    stream.read_to_string(&mut clients_str).unwrap();
    let clients: Vec<Client> = serde_json::from_str(&clients_str)?;

    // If there aren't any clients, we don't want to
    // stop the application from functioning
    if clients.is_empty() {
        return Ok(false);
    }

    Ok(clients
        .into_iter()
        .any(|client| client.focus_history_id == 0 && (client.fullscreen || client.floating)))
}

fn main() {
    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap();
    let hyprland_instance_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap();
    let opts = Opts::parse();
    let socket_one = format!(
        "{}/hypr/{}/.socket.sock",
        xdg_runtime_dir, hyprland_instance_signature
    );
    let mut layers = get_layers(&opts.namespace, &socket_one).unwrap();

    while layers.len() != opts.namespace.len() {
        sleep(std::time::Duration::from_secs(1));
        layers = get_layers(&opts.namespace, &socket_one).unwrap();
    }

    let cursorpos = Arc::new(RwLock::new(get_cursor_pos(&socket_one).unwrap()));

    let cursorpos_updater = Arc::clone(&cursorpos);
    let cursorpos_update_notifier = Arc::new((Mutex::new(false), Condvar::new()));

    layers.into_iter().for_each(|mut layer| {
        let cursorpos = Arc::clone(&cursorpos);
        let notifier = Arc::clone(&cursorpos_update_notifier);
        thread::spawn(move || loop {
            dbg!("Woke up to do update");
            {
                let curr_pos = cursorpos.read().unwrap();

                match layer.toggle_visibility(&curr_pos) {
                    Ok(_) => {}
                    Err(err) => eprintln!("{}", err),
                }
            }
            let lock = notifier.0.lock().unwrap();
            let _guard = notifier.1.wait(lock).unwrap();
        });
    });

    let notifier = Arc::clone(&cursorpos_update_notifier);

    loop {
        thread::sleep(time::Duration::from_millis(100));
        dbg!("Checking cursor pos");

        if fullscreen_or_floating_focused(&socket_one).is_ok_and(|res| res) {
            continue;
        }

        let mut prev_pos = cursorpos_updater.write().unwrap();

        let curr_pos = match get_cursor_pos(&socket_one) {
            Ok(new_cursorpos) => new_cursorpos,
            Err(err) => {
                eprintln!("{}", err);
                continue;
            }
        };

        if *prev_pos == curr_pos {
            continue;
        }

        *prev_pos = curr_pos;
        notifier.1.notify_all();
    }
}
