/*
   Copyright 2019 Supercomputing Systems AG

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.

*/
extern crate substrate_api_client;

#[macro_use]
extern crate log;
extern crate env_logger;

// This module depends on node_runtime.
// To avoid dependency collisions, node_runtime has been removed from the substrate-api-client library.
// Replace this crate by your own if you run a custom substrate node to get your custom events
use node_runtime::Event;

use substrate_api_client::{Api, hexstr_to_vec};
use parity_codec::Decode;
use std::sync::mpsc::channel;
use std::thread;

fn main() {
    env_logger::init();

    let mut api = Api::new("ws://127.0.0.1:9944");
    api.init();

    let (events_in, events_out) = channel();

    println!("Subscribe to events");

    let _eventsubscriber = thread::Builder::new()
            .name("eventsubscriber".to_owned())
            .spawn(move || {
                api.subscribe_events(events_in.clone());
            })
            .unwrap();

    loop {
        let event_str = events_out.recv().unwrap();

        let mut _unhex = hexstr_to_vec(event_str);
        let _events = Vec::<system::EventRecord::<Event>>::decode(&mut _unhex);
        match _events {
            Some(evts) => {
                for evr in &evts {
                    debug!("decoded: phase {:?} event {:?}", evr.phase, evr.event);
                    match &evr.event {
                        Event::balances(be) => {
                            println!(">>>>>>>>>> balances event: {:?}", be);
                            match &be {
                                balances::RawEvent::Transfer(transactor, dest, value, fee) => {
                                    println!("Transactor: {:?}", transactor);
                                    println!("Destination: {:?}", dest);
                                    println!("Value: {:?}", value);
                                    println!("Fee: {:?}", fee);
                                    },
                                _ => {
                                    debug!("ignoring unsupported balances event");
                                    },
                            }},
                        _ => {
                            debug!("ignoring unsupported module event: {:?}", evr.event)
                            },
                    }
                }
            }
            None => error!("couldn't decode event record list")
        }
    }
}