use std::rc::Rc;

use wasm_bindgen::{prelude::{wasm_bindgen, Closure}, JsCast, JsValue};
#[cfg(not(feature = "generic_patch"))]
use wasm_bindgen_futures::JsFuture;
use web_sys::{File, HtmlInputElement, Worker, MessageEvent};
#[cfg(not(feature = "generic_patch"))]
use web_sys::{Response, Blob};
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
    #[wasm_bindgen(js_name = "downloadIso", catch)]
    pub async fn download_iso(is_wii: bool) -> Result<(), JsValue>;
}

pub struct App {
    worker: Rc<Worker>,
    is_patching: Rc<bool>,
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

    fn create(ctx: &Context<Self>) -> Self {
        // TODO Create a worker and send it a MessageChannel
        let worker = Rc::new(Worker::new("app_worker.js").expect("Could not create the worker"));

        let callback: Callback<Message> = ctx.link().callback(|msg| msg);
        let closure = Closure::wrap(Box::new(move |event: MessageEvent| {
            web_sys::console::info_1(&event);
            let data = event.data();
            let type_ = match js_sys::Reflect::get(&data, &"type".into()) {
                Ok(type_) => type_,
                Err(err) => {web_sys::console::warn_1(&err); return;},
            };
            if type_.as_string().map_or(false, |s| &s == "done") {
                let is_wii = match js_sys::Reflect::get(&data, &"is_wii".into()) {
                    Ok(is_wii) => is_wii.as_bool().unwrap_or(false),
                    Err(err) => {web_sys::console::warn_1(&err); return;},
                };
                callback.emit(Message::PatchedIso);
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(err) = download_iso(is_wii).await {
                        web_sys::console::warn_1(&err);
                    }
                });
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        worker.set_onmessage(Some(&closure.into_js_value().dyn_into().expect("Cannot convert Closure to Function")));

        Self { worker, is_patching: Rc::new(false) }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        log::info!("{:?}", msg);
        match msg {
            Message::PatchIso(patch, iso) => {
                // TODO Send the data to the Worker
                let worker = self.worker.clone();
                if let Some(is_patching) = Rc::get_mut(&mut self.is_patching) {
                    *is_patching = true;
                }
                wasm_bindgen_futures::spawn_local(async move {
                    let obj = js_sys::Object::new();
                    if js_sys::Reflect::set(&obj, &"type".into(), &"run".into()).is_err() {
                        return;
                    }
                    #[cfg(not(feature = "generic_patch"))]
                    {
                        let resp = JsFuture::from(web_sys::window().unwrap().fetch_with_str(&patch.uri)).await;
                        let resp: Response = match resp {
                            Ok(resp) => resp.dyn_into().unwrap(),
                            Err(err) => {web_sys::console::warn_1(&err);return;},
                        };
                        let blob: Blob = JsFuture::from(resp.blob().unwrap()).await.unwrap().dyn_into().unwrap();
                        if js_sys::Reflect::set(&obj, &"patch".into(), &blob).is_err() {
                            return;
                        }
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
                true
            }
            Message::PatchError => {
                log::info!("PatchError");
                true
            }
            Message::PatchedIso => {
                log::info!("PatchedIso");
                if let Some(is_patching) = Rc::get_mut(&mut self.is_patching) {
                    *is_patching = false;
                }
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        html! {
            <MainForm patch_callback={ctx.link().callback(move |(patch, save)| Message::PatchIso(patch, save))} is_patching={*self.is_patching}></MainForm>
        }
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
    pub disabled: Option<bool>,
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
        <><label for="patch">{"Patch File: "}</label><input type="file" accept=".patch" id="patch" disabled={props.disabled.unwrap_or(false)} onchange={onchange}/></>
    }
}

#[cfg(not(feature = "generic_patch"))]
#[function_component]
pub fn PatchInput(props: &PatchInputProps) -> Html {
    use web_sys::{HtmlOptionElement, HtmlSelectElement};

    let patches: Vec<Patch> = vec![
        Patch {
            name: "GC NTSCU".into(),
            uri: "patches/0.5.0-gcn-ntscu.patch".into(),
            version: "gcn_ntscu".into(),
        },
        Patch {
            name: "Wii NTSCU 1.0".into(),
            uri: "patches/0.5.0-wii-ntscu-10.patch".into(),
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
            <select id="patch" disabled={props.disabled.unwrap_or(false)} onchange={onchange}>
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
    disabled: Option<bool>,
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
            <input id="iso_in" accept=".iso" type="file" disabled={props.disabled.unwrap_or(false)} onchange={onchange}/>
        </>
    }
}

#[derive(Debug, Properties, PartialEq)]
pub struct MainFormProps {
    patch_callback: Callback<(Patch, Iso)>,
    is_patching: bool,
}

#[function_component]
pub fn MainForm(props: &MainFormProps) -> Html {
    let is_patching = props.is_patching;
    let selected_patch = use_state(|| <Option<Patch>>::None);
    let selected_iso = use_state(|| <Option<Iso>>::None);
    let callback = {
        let selected_iso = selected_iso.clone();
        let selected_patch = selected_patch.clone();
        let patch_callback = props.patch_callback.clone();
        Callback::from(move |_| {
            log::info!("Clicked the patch button");
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
    let iso_change_callback = {
        let selected_iso = selected_iso.clone();
        Callback::from(move |iso| {
            selected_iso.set(iso);
        })
    };
    html! {
        <fieldset id="main_form">
            <legend>{"ISO Patcher"}</legend>
            <PatchInput callback={patch_input_callback} disabled={is_patching} />
            <IsoInput callback={iso_change_callback} disabled={is_patching} />
            <label/><button disabled={is_patching || selected_patch.is_none() || selected_iso.is_none()} onclick={callback}>{"Patch"}</button>
            <StatusBar msg={if is_patching {Some("Patching...")} else {None}}/>
        </fieldset>
    }
}

#[derive(Debug, Properties, PartialEq)]
pub struct StatusBarProps {
    msg: Option<String>,
    progress: Option<f64>,
}

#[function_component]
fn StatusBar(props: &StatusBarProps) -> Html {
    let msg = props.msg.clone();
    let progress = props.progress;
    html! {
        <>
            if msg.is_some() || progress.is_some() {
                if let Some(msg) = msg {
                    <label for="progress_bar">{msg}</label>
                }
                if let Some(progress) = progress {
                    <progress id="progress_bar" max="100" value={format!("{progress}")}/>
                } else {
                    <progress id="progress_bar" max="100"/>
                }
            }
        </>
    }
}