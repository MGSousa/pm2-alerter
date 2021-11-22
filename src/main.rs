extern crate regex;
extern crate zbx_sender;

use std::{borrow::Borrow, str};
use zbx_sender::{Response, Result, Sender};

use bstr::ByteVec;
use clap::Parser;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{Result as Sresult, Value};
use std::io::prelude::*;
use std::os::unix::net::UnixStream;

const ZABBIX_HOST: &str = "localhost";
const PROCESS_EVENT: &str = "s:process:event";
const LOG_EVENT: &str = "s:log:PM2";
const RESTART_EVENT: &str = "restarted because it exceeds --max-memory-restart";
const EVENT_TYPE: &str = "\"event\":\"online\"";

#[derive(Parser)]
#[clap(name = "pm2-alerter")]
struct Alerter {
    /// Zabbix Agent Host url
    #[clap(short, long)]
    host: String,

    /// Zabbix Agent Host port
    #[clap(short, long, default_value = "10051")]
    port: u16,

    /// Unix Socket to connect
    #[clap(short, long)]
    socket: String,

    /// Zabbix event to be sent
    #[clap(short, long, default_value = "pm2.events")]
    event: String,
}

/// Find Process ID from text
fn find_process(text: &str) -> i64 {
    lazy_static! {
        static ref RE: Regex =
            Regex::new(r"Process (?P<id>\d+) restarted because it exceeds --max-memory-restart")
                .unwrap();
    }
    match RE.captures(text) {
        Some(x) => x.name("id").unwrap().as_str().parse::<i64>().unwrap(),
        None => {
            println!("No match!!");
            0
        }
    }
}

/// Parse log object
fn parse_obj(s: &str) -> Sresult<Value> {
    let p: Value = serde_json::from_str(s)?;
    Ok(p)
}

/// Send event alert to Zabbix
fn send_alert(host: &str, port: u16, event: &str, val: &str) -> Result<Response> {
    let sender = Sender::new(ZABBIX_HOST.to_owned(), port);
    sender.send((host, event, val))
}

fn main() {
    let mut pid = -1;
    let alerter = Alerter::parse();

    let mut socket = UnixStream::connect(alerter.socket).unwrap();
    let mut resp = vec![0; 6192];

    println!("Listening events on socket");

    loop {
        match socket.read(&mut resp) {
            Ok(v) => {
                // lossily converts bytes into OS strings
                let content = Vec::from_slice(&resp[0..v]).into_os_string_lossy();
                let s = content.to_string_lossy();
                let str = s.trim_matches(char::from(0));

                // check for event logs to find process ID
                if str.contains(LOG_EVENT) {
                    if str.contains(RESTART_EVENT) {
                        pid = find_process(str);
                    }
                }

                // check for process events
                if str.contains(PROCESS_EVENT) {
                    if str.contains(EVENT_TYPE) {
                        let str_opt = str.split_once(":{").unwrap().1;
                        let mut s = str_opt.to_string();
                        s.insert_str(0, "{");

                        // try to parse it
                        match parse_obj(s.borrow()) {
                            Ok(v) => {
                                match v["process"]["pm_id"].as_i64() {
                                    None => {}
                                    Some(id) => {
                                        if pid == id {
                                            let service = v["process"]["name"]
                                                .as_str()
                                                .unwrap();

                                            println!("{} restarted", service);
                                            pid = -1;

                                            match send_alert(
                                                alerter.host.borrow(),
                                                alerter.port,
                                                alerter.event.borrow(),
                                                service,
                                            ) {
                                                Ok(response) => {
                                                    println!("{:?}", response)
                                                }
                                                Err(e) => println!("error {:?}", e),
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                println!("{}", e)
                            }
                        }
                    }
                }
            }
            Err(r) => {
                println!("socket bus error: {:?}", r);
            }
        }
    }
}
