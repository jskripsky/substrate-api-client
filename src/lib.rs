
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
#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate log;

use ws::{connect, Handler, Sender, Handshake, Result, Message, CloseCode};
use hex;
use serde_json::{json};

use parity_codec::Decode;
use node_primitives::Hash;
use primitives::{twox_128, blake2_256};
use primitive_types::U256;

use metadata::{RuntimeMetadata, RuntimeMetadataPrefixed};

use std::sync::mpsc::Sender as ThreadOut;
use std::sync::mpsc::channel;
use std::thread;

const REQUEST_TRANSFER: u32         = 3;

#[derive(Serialize, Deserialize, Debug)]
struct JsonBasic {
    jsonrpc: String,
    method: String,
    params: String,
}

#[derive(Debug)]
pub struct Api {
    url : &str,
    pub genesis_hash : Option<Hash>,
    //pub metadata : Option<RuntimeMetadataV4>,
}

impl Api {
    pub fn new(url: &str) -> Api {
        Api {
            url: url,
            genesis_hash: None,
//            metadata : None,
        }
    }

    pub fn init(&mut self) {
        // get genesis hash
        let jsonreq = json!({
            "method": "chain_getBlockHash",
            "params": [0],
            "jsonrpc": "2.0",
            "id": "1",
        });
        let genesis_hash_str = self.get_request(jsonreq.to_string()).unwrap();
        self.genesis_hash = Some(hexstr_to_hash(genesis_hash_str));
        info!("got genesis hash: {:?}", self.genesis_hash.unwrap());

        //get metadata
        let jsonreq = json!({
            "method": "state_getMetadata",
            "params": null,
            "jsonrpc": "2.0",
            "id": "1",
        });
        let metadata_str = self.get_request(jsonreq.to_string()).unwrap();
        let _unhex = hexstr_to_vec(metadata_str);
        let mut _om = _unhex.as_slice();
        let _meta = RuntimeMetadataPrefixed::decode(&mut _om)
                .expect("runtime metadata decoding to RuntimeMetadataPrefixed failed.");
        debug!("decoded: {:?} ", _meta);
        match _meta.1 {
            RuntimeMetadata::V4(_value) => {
                //FIXME: storing metadata in self is problematic because it can't be cloned or synced among threads
                //self.metadata = Some(value);
                debug!("successfully decoded metadata");
            },
            _ => panic!("unsupported metadata"),
        }


/*                    match value.modules {
                        DecodeDifferent::Decoded(mods) => {
                            modules = mods;
                            println!("module0 {:?}", modules[0]);
                        },
                        _ => panic!("unsupported metadata"),
                    }

            println!("-------------------- modules ----------------");
            for module in modules {
                println!("module: {:?}", module.name);
                match module.name {
                    DecodeDifferent::Decoded(name) => {
                        match module.calls {
                            Some(DecodeDifferent::Decoded(calls)) => {
                                println!("calls: {:?}", calls);
                            },
                            _ => println!("ignoring"),
                        }
                        println!("storage: {:?}", module.storage)
                    },
                    _ => println!("ignoring"),
                }
            }
            */
    }

    // low level access
    pub fn get_request(&self, jsonreq: String) -> Result<String> {
        let (result_in, result_out) = channel();
        let _url = self.url.clone();
        let _client = thread::Builder::new()
            .name("client".to_owned())
            .spawn(move || {
                connect(_url, |out| {
                    Getter {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                    }
                }).unwrap()
            })
            .unwrap();
        Ok(result_out.recv().unwrap())
    }

    pub fn get_storage(&self, module: &str, storage_key_name: &str, param: Option<Vec<u8>>) -> Result<String> {
        let keyhash = storage_key_hash(module, storage_key_name, param);

        debug!("with storage key: {}", keyhash);
        let jsonreq = json!({
            "method": "state_getStorage",
            "params": [keyhash],
            "jsonrpc": "2.0",
            "id": "1",
        });
        self.get_request(jsonreq.to_string())

    }

    pub fn send_extrinsic(&self, xthex_prefixed: String) -> Result<Hash> {
        debug!("sending extrinsic: {:?}", xthex_prefixed);

        let jsonreq = json!({
            "method": "author_submitAndWatchExtrinsic",
            "params": [xthex_prefixed],
            "jsonrpc": "2.0",
            "id": REQUEST_TRANSFER.to_string(),
        }).to_string();

        let (result_in, result_out) = channel();
        let _url = self.url.clone();
        let _client = thread::Builder::new()
            .name("client".to_owned())
            .spawn(move || {
                connect(_url, |out| {
                    ExtrinsicHandler {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                    }
                }).unwrap()
            })
            .unwrap();
        Ok(result_out.recv().unwrap())
    }

    pub fn subscribe_events(&self, sender: ThreadOut<String>) {
        debug!("subscribing to events");
        let key = storage_key_hash("System", "Events", None);
        let jsonreq = json!({
            "method": "state_subscribeStorage",
            "params": [[key]],
            "jsonrpc": "2.0",
            "id": "1",
        }).to_string();

        let (result_in, result_out) = channel();
        let _url = self.url.clone();
        let _client = thread::Builder::new()
            .name("client".to_owned())
            .spawn(move || {
                connect(_url, |out| {
                    SubscriptionHandler {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                    }
                }).unwrap()
            })
            .unwrap();

        loop {
            let res = result_out.recv().unwrap();
            sender.send(res.clone()).unwrap();

/*
            //println!("client >>>> got {}", res);
            let _unhex = hexstr_to_vec(res);
            let mut _er_enc = _unhex.as_slice();
            //let _event = balances::RawEvent::decode(&mut _er_enc2);
            let _events = Vec::<system::EventRecord::<node_runtime::Event>>::decode(&mut _er_enc);
            match _events {
                Some(evts) => {
                    for ev in &evts {
                        println!("decoded: phase {:?} event {:?}", ev.phase, ev.event);
                        sender.send(ev.event.clone()).unwrap();
                    }
                }
                None => println!("couldn't decode event record list")
            }
            //self.result.send(_events).unwrap();
*/
        }
    }
}

struct Getter {
    out: Sender,
    request: String,
    result: ThreadOut<String>,
}

impl Handler for Getter {
    fn on_open(&mut self, _: Handshake) -> Result<()> {

        info!("sending request: {}", self.request);
        self.out.send(self.request.clone()).unwrap();
        Ok(())
    }
    fn on_message(&mut self, msg: Message) -> Result<()> {
        info!("got message");
        debug!("{}", msg);
        let retstr = msg.as_text().unwrap();
        let value: serde_json::Value = serde_json::from_str(retstr).unwrap();

        // FIXME: defaulting zo zero can be problematic. better to use Option<String>
        let hexstr = match value["result"].as_str() {
                        Some(res) => res.to_string(),
                        _ => "0x00".to_string(),
        };
        self.result.send(hexstr).unwrap();
        self.out.close(CloseCode::Normal).unwrap();
        Ok(())
    }
}

struct SubscriptionHandler {
    out: Sender,
    request: String,
    result: ThreadOut<String>,
}

impl Handler for SubscriptionHandler {
    fn on_open(&mut self, _: Handshake) -> Result<()> {

        info!("sending request: {}", self.request);
        self.out.send(self.request.clone()).unwrap();
        Ok(())
    }
    fn on_message(&mut self, msg: Message) -> Result<()> {
        info!("got message");
        debug!("{}", msg);
        let retstr = msg.as_text().unwrap();
        let value: serde_json::Value = serde_json::from_str(retstr).unwrap();
        match value["id"].as_str() {
            Some(_idstr) => { },
            _ => {
                // subscriptions
                debug!("no id field found in response. must be subscription");
                debug!("method: {:?}", value["method"].as_str());
                match value["method"].as_str() {
                    Some("state_storage") => {
                        let _changes = &value["params"]["result"]["changes"];
                        let _res_str = _changes[0][1].as_str().unwrap().to_string();
                        self.result.send(_res_str).unwrap();
                    }
                    _ => error!("unsupported method"),
                }
            },
        };
        Ok(())
    }
}

struct ExtrinsicHandler {
    out: Sender,
    request: String,
    result: ThreadOut<Hash>,
}

impl Handler for ExtrinsicHandler {
    fn on_open(&mut self, _: Handshake) -> Result<()> {
        info!("sending request: {}", self.request);
        self.out.send(self.request.clone()).unwrap();
        Ok(())
    }

    fn on_message(&mut self, msg: Message) -> Result<()> {
        info!("got message");
        debug!("{}", msg);
        let retstr = msg.as_text().unwrap();
        let value: serde_json::Value = serde_json::from_str(retstr).unwrap();
        match value["id"].as_str() {
            Some(idstr) => { match idstr.parse::<u32>() {
                Ok(REQUEST_TRANSFER) => {
                    match value.get("error") {
                        Some(err) => error!("ERROR: {:?}", err),
                        _ => debug!("no error"),
                    }
                },
                Ok(_) => debug!("unknown request id"),
                Err(_) => error!("error assigning request id"),
            }},
            _ => {
                // subscriptions
                debug!("no id field found in response. must be subscription");
                debug!("method: {:?}", value["method"].as_str());
                match value["method"].as_str() {
                    Some("author_extrinsicUpdate") => {
                        match value["params"]["result"].as_str() {
                            Some(res) => debug!("author_extrinsicUpdate: {}", res),
                            _ => {
                                debug!("author_extrinsicUpdate: finalized: {}", value["params"]["result"]["finalized"].as_str().unwrap());
                                // return result to calling thread
                                self.result.send(hexstr_to_hash(value["params"]["result"]["finalized"].as_str().unwrap().to_string())).unwrap();
                                // we've reached the end of the flow. return
                                self.out.close(CloseCode::Normal).unwrap();
                            },
                        }
                    }
                    _ => error!("unsupported method"),
                }
            },
        };
        Ok(())
    }
}

pub fn storage_key_hash(module: &str, storage_key_name: &str, param: Option<&str>) -> String {
    let key = format!("{} {}", module, storage_key_name);
    let mut hash = hex::encode(
        match param {
            Some(par) => blake2_256(&format!("{}{}", key, par));
            _ => twox_128(&key);
        }
    );
    hash.insert_str(0, "0x");
    hash
}

pub fn hexstr_to_vec(hexstr: &str) -> Vec<u8> {
    if hexstr.starts_with("0x") {
        hexstr = &hexstr[2..];
    } else {
        info!("converting non-prefixed hex string");
    }
    hex::decode(hexstr).unwrap()
}

pub fn hexstr_to_u256(hexstr: &str) -> U256 {
    let bytes = hexstr_to_vec(hexstr);
    U256::from_little_endian(&bytes)
}

pub fn hexstr_to_hash(hexstr: &str) -> Hash {
    let bytes = hexstr_to_vec(hexstr);
    Hash::from(&bytes)
}



