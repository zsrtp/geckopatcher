importScripts("worker.js");

wasm_bindgen("worker_bg.wasm").then((_) => {
    globalThis.addEventListener("message", (event) => {
        switch (event.data.type) {
            case "run": {
                wasm_bindgen.run_patch(event.data.patch, event.data.file, event.data.save);
                break;
            }
            default: {
                console.warn("Unknown message type:", event.data.type);
                break;
            }
        }
    });
    console.debug("Registered message listener");
});
