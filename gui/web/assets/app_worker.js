importScripts("worker.js");

async function registerLocalStorage(patch, iso) {
    const root = await navigator.storage.getDirectory();
    let patch_ = await root.getFileHandle("in.patch", { create: true });
    let patchWritable = await patch_.createWritable();
    await patchWritable.truncate(0);
    await patch.stream().pipeTo(patchWritable);
    let iso_ = await root.getFileHandle("in.iso", { create: true });
    let isoWritable = await iso_.createWritable();
    await isoWritable.truncate(0);
    await iso.stream().pipeTo(isoWritable);
    const fileHandle = await root.getFileHandle("tpgz.iso", { create: true });
    return [patch_, iso_, fileHandle];
}

async function deleteLocalStorage(patch, iso, save) {
    let patchWritable = await patch.createWritable();
    await patchWritable.truncate(0);
    await patchWritable.close();
    let isoWritable = await iso.createWritable();
    await isoWritable.truncate(0);
    await isoWritable.close();
    let saveWritable = await save.createWritable();
    await saveWritable.truncate(0);
    await saveWritable.close();
}

wasm_bindgen("worker_bg.wasm").then((_) => {
    globalThis.addEventListener("message", (event) => {
        switch (event.data.type) {
            case "run": {
                registerLocalStorage(event.data.patch, event.data.file).then(([patch, file, save]) => {
                    wasm_bindgen.run_patch(patch, file, save);
                    // return deleteLocalStorage(patch, file, save);
                }).then(() => postMessage({ type: "finished" }));
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
