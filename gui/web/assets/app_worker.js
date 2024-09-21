importScripts("worker.js");

async function registerLocalStorage(patch, iso) {
    const root = await navigator.storage.getDirectory();
    let patch_ = await root.getFileHandle("in.patch", { create: true });
    let patchWritable = patch_.createWritable();
    let patchRet = patchWritable
        .then((patchWritable_) => patchWritable_.truncate(0))
        .then(() => patchWritable)
        .then((patchWritable_) => patch.stream().pipeTo(patchWritable_))
        .then(() => patch_);
    let iso_ = await root.getFileHandle("in.iso", { create: true });
    let isoWritable = iso_.createWritable();
    let isoRet = isoWritable
        .then((isoWritable_) => isoWritable_.truncate(0))
        .then(() => isoWritable)
        .then((isoWritable_) => iso.stream().pipeTo(isoWritable_))
        .then(() => iso_);
    let fileHandle_ = await root.getFileHandle("out.iso", { create: true });
    let fileWritable = fileHandle_.createWritable();
    let fileRet = fileWritable
        .then((fileHandle) => fileHandle.truncate(0))
        .then(() => fileWritable)
        .then((fileWritable) => fileWritable.close())
        .then(() => fileHandle_);
    return await Promise.all([patchRet, isoRet, fileRet]);
}

async function deleteLocalStorage(patch, iso) {
    let patchPromise = patch.createWritable().then(async (patchWritable) => {
        await patchWritable.truncate(0);
        return patchWritable.close();
    });
    let isoPromise = iso.createWritable().then(async (isoWritable) => {
        await isoWritable.truncate(0);
        return isoWritable.close();
    });
    return await Promise.all([patchPromise, isoPromise]);
}

let is_running = false;

wasm_bindgen("worker_bg.wasm").then((_) => {
    globalThis.addEventListener("message", (event) => {
        switch (event.data.type) {
            case "run": {
                if (!is_running) {
                    is_running = true;
                    globalThis.postMessage({ type: "progress", title: "Loading Files..." });
                    registerLocalStorage(event.data.patch, event.data.file).then(([patch, file, save]) => {
                        console.dir(patch, file, save);
                        return wasm_bindgen.run_patch(patch, file, save).then((filename) => [[patch, file], filename]);
                    })
                        .then(async ([[patch, file], filename]) => {
                            let f = await file.getFile();
                            return Promise.all([filename, deleteLocalStorage(patch, file)]);
                        })
                        .then(([filename,]) => {
                            console.debug("Done", filename);
                            globalThis.postMessage({ type: "done", filename: filename });
                        })
                        .catch((err) => {
                            globalThis.postMessage({ type: "cancelled", msg: err });
                            globalThis.postMessage({ type: "progress", title: err });
                            throw err;
                        })
                        .finally(() => {
                            is_running = false;
                        });
                }
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
