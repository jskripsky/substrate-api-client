
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

use std::sync::mpsc::{Sender as ThreadOut, channel};
use std::thread;

const REQUEST_TRANSFER: u32         = 3;

#[derive(Debug)]
pub struct Api {
    url: &str,
    pub genesis_hash: Option<Hash>,
}

impl Api {
    pub fn new(url: &str) -> Api {
        Api {
            url: url,
            genesis_hash: None,
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
        let mut _unhex = hexstr_to_vec(metadata_str);
        let (_, _meta) = RuntimeMetadataPrefixed::decode(&mut _unhex)
                .expect("runtime metadata decoding to RuntimeMetadataPrefixed failed.");
        debug!("decoded: {:?} ", _meta);
        match _meta {
            RuntimeMetadata::V4(_value) => {
                //FIXME: storing metadata in self is problematic because it can't be cloned or synced among threads
                //self.metadata = Some(value);
                debug!("successfully decoded metadata");
            },
            _ => panic!("unsupported metadata"),
        }
    }

    // low level access
    pub fn get_request(&self, jsonreq: String) -> Result<String> {
        let (result_in, result_out) = channel();
        let _client = thread::Builder::new()
            .name("client".to_owned())
            .spawn(move || {
                connect(url, |out| {
                    GenericHandler {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                        processor: &getter
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
                    GenericHandler {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                        processor: &extrinsicHandler
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
                    GenericHandler {
                        out: out,
                        request: jsonreq.clone(),
                        result: result_in.clone(),
                        processor: &subscriptionHandler
                    }
                }).unwrap()
            })
            .unwrap();

        loop {
            let res = result_out.recv().unwrap();
            sender.send(res.clone()).unwrap();
        }
    }
}

struct GenericHandler<T> {
    out: Sender,
    request: String,
    result: ThreadOut<T>,
    processor: FnMut(&mut self, serde_json::Value) -> Result<()>
}

impl Handler for GenericHandler {
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
        self.processor(value)
    }
}

fn getter (&mut self: GenericHandler<String>, value: serde_json::Value) -> Result<()> {
    let hexstr = match value["result"].as_str() {
        Some(res) => res.to_string(),
        _ => "0x00".to_string(),
    }
    self.result.send(hexstr).unwrap();
    self.out.close(CloseCode::Normal).unwrap();
    Ok(())
)

fn subscriptionHandler (&mut self: GenericHandler<String>, value: serde_json::Value) -> Result<()> {
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
    }
    Ok(())
}

fn extrinsicHandler (&mut self: GenericHandler<Hash>, value: serde_json::Value) -> Result<()> {
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
    }
    Ok(())
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
