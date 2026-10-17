function setupDownload(file, filename) {
    let a = document.createElement("a");
    a.style.display = "none";
    a.download = filename;
    let url = window.URL.createObjectURL(file);
    a.href = url;
    document.body.appendChild(a);
    a.click();
    console.debug("Download done. Cleaning...");
    document.body.removeChild(a);
    window.URL.revokeObjectURL(url);
}

window["downloadIso"] = async (filename) => {
    let root = await navigator.storage.getDirectory();
    let fileHandle = await root.getFileHandle("out.iso");
    setupDownload(await fileHandle.getFile(), filename);
};

let mappings = {};

async function fetchMappings() {
    let response = await fetch("patches/mapping.json");
    let mapping = await response.json();
    let ret = {};
    let maxLength = Object.entries(mapping).reduce((acc, curr) => (acc > curr[0].length ? acc : curr[0].length), 0)
    for (let key in mapping) {
        let val = new Uint8Array(maxLength);
        for (let i = 0; i < Math.min(maxLength, key.length); i++) {
            val[i] = key.charCodeAt(i);
        }
        ret[mapping[key]] = val;
    }
    return ret;
};

window["fetchMappings"] = () => {
    fetchMappings().then((m) => {
        mappings = m;
    });
};

function equalArray(buf1, buf2) {
    if (buf1.byteLength != buf2.byteLength) return false;
    var dv1 = new Int8Array(buf1);
    var dv2 = new Int8Array(buf2);
    for (var i = 0; i != buf1.byteLength; i++) {
        if (dv1[i] != dv2[i]) return false;
    }
    return true;
}

window["getMapping"] = (buf) => {
    let val = Object.entries(mappings).find(([key, val]) => {
        if (equalArray(buf, val)) {
            return key;
        }
    });
    let ret = (val) ? val[0] : "unkown";
    return ret;
};

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);

let worker = null;
let is_worker_ready = false;
let is_patching = false;
let errored = null;
let status_msg = null;
let progress_val = null;

let selected_patch = null; // {name, path, version} in patched mode, a File in generic mode
let selected_iso = null;   // {file, version}
let patches = [];          // parsed patches/meta.json
let is_generic = false;    // true when patches/meta.json is absent
let last_version = null;
let revealed = false;

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

function str(err) {
    if (err instanceof Error) return err.message;
    if (typeof err === "string") return err;
    return String(err);
}

function patchError(error) {
    console.warn("PatchError");
    errored = error;
    is_patching = false;
    status_msg = "Error";
    renderAll();
}

function patchedIso(filename) {
    console.info("PatchedIso");
    is_patching = false;
    renderAll();
    window["downloadIso"](filename).catch((err) => {
        console.warn(err);
    });
}

function workerReady() {
    console.info("Worker ready");
    is_worker_ready = true;
    renderAll();
}

function createWorker() {
    console.info("Initializing application...");
    worker = new Worker("app_worker.js", { type: "module", name: "patcher_thread" });

    worker.onerror = (event) => {
        const message = event.message || "";
        const detail = (event.error && str(event.error)) || "";
        const text = detail ? `${message}\n${detail}` : message;
        console.error(event);
        status_msg = `Error loading app: ${text}`;
        progress_val = 100;
        renderAll();
    };

    worker.onmessage = (event) => {
        const data = event.data;
        switch (data.type) {
            case "ready":
                workerReady();
                break;
            case "cancelled":
                patchError(str(data.msg));
                break;
            case "done":
                patchedIso(data.filename);
                break;
            case "progress":
                status_msg = data.title ?? null;
                progress_val = data.progress ?? null;
                renderAll();
                break;
            default:
                console.warn("Unknown message type:", data.type);
                break;
        }
    };
}

// ---------------------------------------------------------------------------
// Patch selection (patched mode vs generic mode)
// ---------------------------------------------------------------------------

async function loadPatches() {
    let ok = false;
    try {
        const resp = await fetch("patches/meta.json");
        ok = resp.ok;
        if (ok) {
            patches = await resp.json();
        }
    } catch (err) {
        console.warn("Could not load patches/meta.json, falling back to generic mode", err);
        ok = false;
    }
    if (!ok) {
        enableGenericMode();
        return;
    }
    // Show the project link (tpgz) only when shipping with bundled patches.
    const links = $("links");
    const tpgz = document.createElement("a");
    tpgz.href = "https://github.com/zsrtp/tpgz";
    tpgz.innerHTML = '<img class="gh-img" src="github-mark-white.png" alt="GitHub Logo"/>tpgz';
    links.appendChild(tpgz);
    // Force a repopulation of the patch list in case an ISO was picked before
    // the patches finished loading.
    last_version = null;
    renderAll();
}

function populatePatchSelect() {
    const select = $("patch");
    select.replaceChildren();
    const placeholder = document.createElement("option");
    placeholder.disabled = true;
    placeholder.textContent = "Select Patch";
    select.appendChild(placeholder);
    const version = selected_iso ? selected_iso.version : null;
    patches.forEach((p, i) => {
        if (version && p.version === version) {
            const opt = document.createElement("option");
            opt.value = String(i + 1);
            opt.textContent = p.name;
            select.appendChild(opt);
        }
    });
}

function enableGenericMode() {
    console.info("Running in generic patch mode");
    is_generic = true;
    const label = document.querySelector('label[for="patch"]');
    label.textContent = "Patch File: ";
    const select = $("patch");
    const input = document.createElement("input");
    input.id = "patch";
    input.type = "file";
    input.accept = ".patch";
    select.replaceWith(input);
    input.addEventListener("change", (e) => {
        selected_patch = e.target.files[0] ?? null;
        renderAll();
    });
    renderAll();
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

async function read8Bytes(file) {
    const slice = file.slice(0, 8);
    const buf = await slice.arrayBuffer();
    return new Uint8Array(buf);
}

function setupIsoInput() {
    const isoInput = $("iso_in");
    isoInput.addEventListener("change", async (e) => {
        const file = e.target.files[0];
        if (!file) {
            selected_iso = null;
            selected_patch = null;
            last_version = undefined;
            renderAll();
            return;
        }
        const buf = await read8Bytes(file);
        console.debug(buf);
        const version = window["getMapping"](buf);
        selected_iso = { file, version };
        selected_patch = null;
        last_version = undefined;
        renderAll();
    });
}

function setupPatchInput() {
    const select = $("patch");
    select.addEventListener("change", (e) => {
        const idx = parseInt(select.value, 10) - 1;
        selected_patch = patches[idx] ?? null;
        renderAll();
    });
}

function setupPatchButton() {
    const patchBtn = $("patch-btn");
    patchBtn.addEventListener("click", () => {
        if (!selected_patch || !selected_iso) return;
        console.debug("Clicked the patch button");
        is_patching = true;
        errored = null;
        renderAll();
        postRun(selected_patch, selected_iso.file);
    });
}

async function postRun(patch, iso) {
    const obj = { type: "run" };
    try {
        if (is_generic) {
            obj.patch = patch;
        } else {
            const resp = await fetch(patch.path);
            if (!resp.ok) {
                throw new Error(`Could not fetch patch: ${resp.status}`);
            }
            obj.patch = await resp.blob();
        }
        obj.file = iso;
        worker.postMessage(obj);
    } catch (err) {
        console.warn(err);
        patchError(str(err));
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function renderAll() {
    const form = $("main_form");
    const links = $("links");
    const patchBtn = $("patch-btn");
    const isoInput = $("iso_in");
    const patchInput = $("patch");

    if (!revealed) {
        revealed = true;
        const loader = document.querySelector(".lds-ring");
        if (loader) loader.style.display = "none";
        form.style.display = "grid";
        links.style.display = "flex";
    }

    // The form is disabled while the worker is still loading.
    const busy = is_patching || !is_worker_ready;

    isoInput.disabled = busy;

    if (is_generic) {
        patchInput.disabled = busy;
    } else {
        const version = selected_iso ? selected_iso.version : null;
        if (version !== last_version) {
            last_version = version;
            populatePatchSelect();
            // The browser highlights the first option, so keep the state in
            // sync by actually selecting the first matching patch.
            const idx = patches.findIndex((p) => p.version === version);
            selected_patch = idx >= 0 ? patches[idx] : null;
            if (selected_patch) {
                $("patch").value = String(idx + 1);
            }
        }
        const patches_empty = !version || !patches.some((p) => p.version === version);
        patchInput.disabled = busy || !selected_iso || patches_empty;
    }

    patchBtn.disabled = busy || !selected_patch || !selected_iso || errored !== null;
    if (errored !== null) {
        patchBtn.classList.add("errored");
    } else {
        patchBtn.classList.remove("errored");
    }

    renderAfterButton();
}

function renderAfterButton() {
    const patchBtn = $("patch-btn");
    document
        .querySelectorAll(
            "#main_form > label[for=progress_bar], #main_form > span.progress-container, #main_form > span#error_msg"
        )
        .forEach((el) => el.remove());

    const busy = is_patching || !is_worker_ready;
    const show_status = busy;
    if (show_status && (status_msg !== null || progress_val !== null)) {
        const label = document.createElement("label");
        label.htmlFor = "progress_bar";
        if (status_msg !== null) {
            const pre = document.createElement("pre");
            pre.textContent = status_msg;
            label.appendChild(pre);
        }
        patchBtn.insertAdjacentElement("afterend", label);

        const span = document.createElement("span");
        span.className = "progress-container";
        const prog = document.createElement("progress");
        prog.id = "progress_bar";
        prog.max = "100";
        span.appendChild(prog);
        if (progress_val !== null) {
            prog.value = String(progress_val);
            span.appendChild(document.createTextNode(`${progress_val.toFixed(1)}%`));
        }
        label.insertAdjacentElement("afterend", span);
    }

    if (errored !== null) {
        const span = document.createElement("span");
        span.id = "error_msg";
        span.textContent = errored;
        const anchor = document.querySelector("#main_form > span.progress-container") || patchBtn;
        anchor.insertAdjacentElement("afterend", span);
    }
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

window["fetchMappings"]();
setupIsoInput();
setupPatchInput();
setupPatchButton();
fetch("manifest.json")
    .then((resp) => resp.json())
    .then((manifest) => {
        if (manifest.version) {
            const versionEl = $("version");
            if (versionEl) versionEl.textContent = `v${manifest.version}`;
        }
    })
    .catch((err) => console.warn("Could not read version:", err));
status_msg = "Starting Application...";
renderAll();
loadPatches();
createWorker();
