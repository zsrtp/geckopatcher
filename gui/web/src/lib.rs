use std::rc::Rc;

use wasm_bindgen::{prelude::wasm_bindgen, JsCast, JsValue};
use web_sys::{File, HtmlInputElement, Worker};
use yew::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "hasShowSaveFilePicker")]
    pub fn hasShowSaveFilePicker() -> JsValue;
    #[wasm_bindgen(js_name = "getPatch", catch)]
    pub async fn get_patch() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "getIso", catch)]
    pub async fn get_iso() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = "getSave", catch)]
    pub async fn get_save() -> Result<JsValue, JsValue>;
}

pub struct App {
    worker: Rc<Worker>,
}

#[derive(Debug)]
pub enum Message {
    PatchIso(Patch, Iso),
    PatchError,
    PatchedIso,
}

impl Component for App {
    type Message = Message;
    type Properties = ();

    fn create(_ctx: &Context<Self>) -> Self {
        // TODO Create a worker and send it a MessageChannel
        let worker = Rc::new(Worker::new("app_worker.js").expect("Could not create the worker"));

        Self { worker }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        log::info!("{:?}", msg);
        match msg {
            Message::PatchIso(patch, iso) => {
                // TODO Send the data to the Worker
                let worker = self.worker.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let obj = js_sys::Object::new();
                    if js_sys::Reflect::set(&obj, &"type".into(), &"run".into()).is_err() {
                        return;
                    }
                    #[cfg(not(feature = "generic_patch"))]
                    if js_sys::Reflect::set(&obj, &"patch".into(), &patch.uri.into()).is_err() {
                        return;
                    }
                    #[cfg(feature = "generic_patch")]
                    if js_sys::Reflect::set(&obj, &"patch".into(), &patch.file).is_err() {
                        return;
                    }
                    if js_sys::Reflect::set(&obj, &"file".into(), &iso.file).is_err() {
                        return;
                    }
                    worker
                        .post_message(&obj)
                        .expect("Message cannot be sent to worker");
                });
                false
            }
            Message::PatchError => {
                log::info!("PatchError");
                true
            }
            Message::PatchedIso => {
                log::info!("PatchedIso");
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        // if hasShowSaveFilePicker().is_truthy() {
            html! {
                <MainForm patch_callback={ctx.link().callback(move |(patch, save)| Message::PatchIso(patch, save))}></MainForm>
            }
        // } else {
        //     html! {
        //         <>
        //         {"Sorry, but your browser doesn't seem to be supported (does not support "}<pre>{" showSaveFilePicker "}</pre>{")."}
        //         </>
        //     }
        // }
    }
}

#[cfg(feature = "generic_patch")]
#[derive(Debug, Clone)]
pub struct Patch {
    file: File,
}

#[cfg(not(feature = "generic_patch"))]
#[derive(Debug, Clone)]
pub struct Patch {
    name: String,
    uri: String,
    version: String,
}

#[derive(Properties, PartialEq)]
pub struct PatchInputProps {
    pub callback: Callback<Option<Patch>>,
}

#[cfg(feature = "generic_patch")]
#[function_component]
pub fn PatchInput(props: &PatchInputProps) -> Html {
    let onchange = {
        let callback = props.callback.clone();
        Callback::from(move |e: Event| {
            let input = e
                .target()
                .expect("On change event doesn't have a target")
                .dyn_into::<HtmlInputElement>()
                .expect("Target is not an input Element");
            let f = input.files().and_then(|x| x.get(0));
            callback.emit(f.map(|x| Patch { file: x }));
        })
    };
    html! {
        <><label for="patch">{"Patch File: "}</label><input type="file" id="patch" onchange={onchange}/></>
    }
}

#[cfg(not(feature = "generic_patch"))]
#[function_component]
pub fn PatchInput(props: &PatchInputProps) -> Html {
    use web_sys::{HtmlOptionElement, HtmlSelectElement};

    let patches: Vec<Patch> = vec![
        Patch {
            name: "GC NTSCU".into(),
            uri: "...".into(),
            version: "gcn_ntscu".into(),
        },
        Patch {
            name: "Wii NTSCU 1.0".into(),
            uri: "...".into(),
            version: "wii_ntscu_10".into(),
        },
    ];
    let onchange = {
        let callback = props.callback.clone();
        Callback::from(move |e: Event| {
            let node = e
                .target()
                .expect("Change event of the Select node does not have a target");
            let target: &HtmlSelectElement =
                node.dyn_ref().expect("Target is not a Select element");
            let option = target
                .selected_options()
                .get_with_index(0)
                .map(|x| {
                    x.dyn_into::<HtmlOptionElement>()
                        .expect("First selected element is not an option")
                })
                .map(|x| Patch {
                    name: x.label(),
                    uri: "".into(),
                    version: x.value(),
                });
            callback.emit(option);
        })
    };
    let patch_html: Html = patches
        .iter()
        .map(|p| {
            html! {
                <option value={p.version.clone()}>{p.name.clone()}</option>
            }
        })
        .collect();
    html! {
        <>
            <label for="patch">{"Patch to Apply: "}</label>
            <select id="patch" onchange={onchange}>
                <option disabled={true} selected={true} value={""}>{"Select Patch"}</option>
                {patch_html}
            </select>
        </>
    }
}

#[derive(Debug, Clone)]
pub struct Iso {
    file: File,
}

#[derive(Properties, PartialEq)]
pub struct IsoInputProps {
    pub callback: Callback<Option<Iso>, ()>,
}

#[function_component]
pub fn IsoInput(props: &IsoInputProps) -> Html {
    let onchange = {
        let callback = props.callback.clone();
        Callback::from(move |e: Event| {
            let input = e
                .target()
                .expect("On change event doesn't have a target")
                .dyn_into::<HtmlInputElement>()
                .expect("Target is not an input Element");
            let f = input.files().and_then(|x| x.get(0));
            callback.emit(f.map(|x| Iso { file: x }));
        })
    };
    html! {
        <>
            <label for="iso_in">{"ISO to patch: "}</label>
            <input id="iso_in" accept=".iso" type="file" onchange={onchange}/>
        </>
    }
}

#[derive(Debug, Properties, PartialEq)]
pub struct MainFormProps {
    patch_callback: Callback<(Patch, Iso)>,
}

#[function_component]
pub fn MainForm(props: &MainFormProps) -> Html {
    let is_patching = use_state(|| false);
    let selected_patch = use_state(|| <Option<Patch>>::None);
    let selected_iso = use_state(|| <Option<Iso>>::None);
    let callback = {
        let is_patching = is_patching.clone();
        let selected_iso = selected_iso.clone();
        let selected_patch = selected_patch.clone();
        let patch_callback = props.patch_callback.clone();
        Callback::from(move |_| {
            log::info!("Clicked the patch button");
            is_patching.set(true);
            patch_callback.emit((
                selected_patch.as_ref().expect("No Patch selected").clone(),
                selected_iso.as_ref().expect("No ISO selected").clone(),
            ));
        })
    };
    let patch_input_callback: Callback<Option<Patch>> = {
        let selected_patch = selected_patch.clone();
        Callback::from(move |patch: Option<Patch>| {
            selected_patch.set(patch);
        })
    };
    let reset_callback = {
        let is_patching = is_patching.clone();
        Callback::from(move |_| {
            is_patching.set(false);
        })
    };
    let iso_change_callback = {
        let selected_iso = selected_iso.clone();
        Callback::from(move |iso| {
            selected_iso.set(iso);
        })
    };
    html! {
        <fieldset id="main_form">
            <legend>{"ISO Patcher"}</legend>
            <PatchInput callback={patch_input_callback} />
            <IsoInput callback={iso_change_callback}/>
            <label/><button disabled={*is_patching || selected_patch.is_none() || selected_iso.is_none()} onclick={callback}>{"Patch"}</button>
            <label></label><button onclick={reset_callback}>{"Reset"}</button>
        </fieldset>
    }
}
