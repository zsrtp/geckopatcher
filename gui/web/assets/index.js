window["hasShowSaveFilePicker"] = () => {return "showSaveFilePicker" in window;};
window["getPatch"] = async () => {
    let file = null;
    try {
        file = (await window.showOpenFilePicker({types: [{description: "Patch file", accept:{"application/x-patch":[".patch"]}}]}))[0];
    }
    catch (e) {
        console.error(e);
    }
    return file;
};
window["getIso"] = async () => {
    let file = null;
    try {
        file = (await window.showOpenFilePicker({types: [{description: "CD Image", accept:{"application/x-cd-image":[".iso"]}}]}))[0];
    }
    catch (e) {
        console.error(e);
    }
    return file;
};
window["getSave"] = async (name) => {
    let file = null;
    try {
        let suggestedName = "tpgz.iso";
        if (name !== undefined) {
            suggestedName = name;
        }
        file = (await window.showSaveFilePicker({suggestedName, types: [{description: "CD Image", accept:{"application/x-cd-image":[".iso"]}}]}));
    }
    catch (e) {
        console.error(e);
    }
    return file;
};
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

function equalArray (buf1, buf2) {
    if (buf1.byteLength != buf2.byteLength) return false;
    var dv1 = new Int8Array(buf1);
    var dv2 = new Int8Array(buf2);
    for (var i = 0 ; i != buf1.byteLength ; i++)
    {
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