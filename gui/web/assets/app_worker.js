importScripts("worker.js");

async function registerLocalStorage(patch, iso) {
    const root = await navigator.storage.getDirectory();
    let patch_ = await root.getFileHandle("in.patch", { create: true });
    let patchRet = patch_.createWritable()
        .then(async (patchWritable) => {
            await patchWritable.truncate(0);
            await patch.stream().pipeTo(patchWritable);
            return patch_;
        });
    let iso_ = await root.getFileHandle("in.iso", { create: true });
    let isoRet = iso_.createWritable()
        .then(async (isoWritable) => {
            await isoWritable.truncate(0);
            await iso.stream().pipeTo(isoWritable);
            return iso_;
        });
    let save = await root.getFileHandle("tpgz.iso", { create: true });
    let saveRet = save.createWritable()
        .then(async (saveWritable) => {
            await saveWritable.truncate(0);
            await saveWritable.close();
            return save;
        });
    return await Promise.all([patchRet, isoRet, saveRet]);
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
                        console.dir([patch, file, save]);
                        globalThis.postMessage({ type: "progress", title: "Init patcher..." });
                        return wasm_bindgen.run_patch(patch, file, save).then(() => [patch, file, save])
                    }
                    )
                        .then(async ([patch, file, save]) => {
                            let f = await file.getFile();
                            return Promise.all([f.slice(0, 6).text(), deleteLocalStorage(patch, file)]);
                        })
                        .then(([gameCode,]) => {
                            console.debug("Done", gameCode);
                            globalThis.postMessage({ type: "done", is_wii: gameCode.startsWith("R") });
                        })
                        .catch((err) => { globalThis.postMessage({ type: "cancelled" }); throw err; })
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
