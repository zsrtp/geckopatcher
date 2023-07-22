import * as gui from "./gui.js";

let memory = null;
let port = null;

self.onmessage = function (event) {
    if (event.data.type == "wasm_mem") {
        console.debug("Worker");
        console.dir(event.data);
        // memory = event.data.mem;
        port = event.data.port;
        port.addEventListener("message", (evt) => {
            console.debug("Worker channel received message:");
            console.dir(evt);
            port.postMessage("test");
        })
        port.start();
    }
    self.postMessage("global message received");
}