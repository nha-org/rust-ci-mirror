use super::button::Button;
use super::parts::icon;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "dropzone",
        classes: &["dropzone", "dropzone-area", "dropzone-input", "dropzone-text", "dropzone-label", "dropzone-file"],
        tags: &[],
        name: "File upload",
        used: "A file to send, dropped or chosen, named with its size before it goes, what sends it inside: /settings' backup read back, a comment's images and PDFs. Built on rust-ui's Dropzone, which dioxus-components lacks.",
        example: Some(|_| {
            rsx! {
                FileUpload { id: "c-file-upload", name: "file", accept: ".zip", label: drop_words(".zip"), required: true,
                    Button { "Read a backup" }
                }
                FileUpload { id: "c-file-upload-many", name: "files", accept: ".png,.pdf", multiple: true,
                    label: "Attach images or PDFs: drop them here, or choose"
                }
            }
        }),
        library: Some("https://rust-ui.com/docs/components/dropzone"),
        menu: true,
        ..Component::BARE
    }
}

/// A file to send, dropped or chosen: rust-ui's Dropzone (notice in NOTICE-rust-ui) on dioxus-components' Input inside its
/// Label (notice in NOTICE-dioxus-components), which has no drop zone (task settings-upload-styled; @nha, 2026-11-09:
/// « unstyled chose file, no design system, does not follow button rules where the button is inside the alement (input or
/// other) »). Kept from rust-ui: a dashed area, an upload icon and its words, a tint while a file is dragged over it, the
/// chosen file named with its size before anything is sent. What sends it (`children`, /settings' submit) sits inside the
/// zone, never beside it. The browser's own input covers the area, transparent, so a click or a drop on it works with no
/// script; DROPZONE_JS, which it carries, names the file and takes a drop anywhere in the zone. ponytail: no list with
/// a remove per file nor previews (rust-ui's DropzoneFileList); add when a page sends several files to review first.
#[component]
pub fn FileUpload(
    #[props(into)] id: Option<String>,
    #[props(into)] name: String,
    #[props(into)] accept: String,
    #[props(into)] label: String,
    #[props(default)] multiple: bool,
    #[props(default)] required: bool,
    children: Element,
) -> Element {
    let (multiple, required) = (multiple.then_some(true), required.then_some(true));
    rsx! {
        div { id, class: "dropzone",
            label { class: "dropzone-area",
                input { class: "dropzone-input", r#type: "file", name, accept, multiple, required }
                {icon(UPLOAD)}
                span { class: "dropzone-text",
                    span { class: "dropzone-label", "{label}" }
                    span { class: "dropzone-file", "aria-live": "polite", "No file chosen" }
                }
            }
            {children}
            script { dangerous_inner_html: DROPZONE_JS }
        }
    }
}

/// FileUpload's words for a file of `accept`, its types joined by « or »: /settings' restore and its /design example say the
/// same (task settings-upload-styled).
pub fn drop_words(accept: &str) -> String {
    let types: Vec<&str> = accept.split(',').map(str::trim).collect();
    format!("Drop a {} file here, or choose one", types.join(" or "))
}

/// FileUpload's script, bound once on the document, so a zone a script clones (the compass's box) works too: every zone
/// carries it, as the menu does, and a page without either gets the browser's own picker under the words (/code-review).
/// A zone is tinted while files, not text or a link, are over it: a count of enters and leaves, as rust-ui's, so its own
/// parts do not flicker it. A drop anywhere in it takes the files its `accept` lists, added to those chosen when it takes
/// several, else the first alone; none taken, it says so and keeps what it had. Each chosen file is named with its size,
/// as rust-ui's `size_display`, its unit the one its rounding stays under 1024 in.
/// Blocks its port: no dioxus-liveview runs, so a web page is a dioxus-ssr picture with no handlers
/// (rfc-rust-unified#ui-lib's drift; liveview's own future is open upstream,
/// https://github.com/DioxusLabs/dioxus/issues/2699); dropped files do not reach a liveview handler
/// (https://github.com/DioxusLabs/dioxus/issues/2930).
pub const DROPZONE_JS: &str = r#"(()=>{if(window.dashDropzone)return;window.dashDropzone=1;
const zone=e=>{const z=e.target.closest&&e.target.closest('.dropzone');return z&&z.querySelector('.dropzone-input')?z:null};
const files=e=>[...(e.dataTransfer&&e.dataTransfer.types||[])].includes('Files');
const size=b=>{if(b<1024)return b+' B';let i=-1;do{b/=1024;i++}while(+b.toFixed(2)>=1024&&i<2);return b.toFixed(2)+' '+['KB','MB','GB'][i]};
const takes=(i,f)=>{const a=i.accept.split(',').map(t=>t.trim().toLowerCase()).filter(Boolean),n=f.name.toLowerCase(),k=(f.type||'').toLowerCase();
return !a.length||a.some(t=>t[0]==='.'?n.endsWith(t):t.endsWith('/*')?k.startsWith(t.slice(0,-1)):k===t)};
const say=(z,t)=>{z.querySelector('.dropzone-file').textContent=t};
const show=z=>{const i=z.querySelector('.dropzone-input');say(z,i.files.length?[...i.files].map(f=>f.name+', '+size(f.size)).join('; '):'No file chosen')};
const calm=z=>{z.dataset.over=0;z.removeAttribute('data-dragging')};
document.addEventListener('change',e=>{const z=zone(e);if(z&&e.target.matches('.dropzone-input'))show(z)});
document.addEventListener('dragenter',e=>{const z=zone(e);if(z&&files(e)){z.dataset.over=(+z.dataset.over||0)+1;z.setAttribute('data-dragging','')}});
document.addEventListener('dragleave',e=>{const z=zone(e);if(z&&files(e)&&(z.dataset.over=(+z.dataset.over||1)-1)<1)calm(z)});
document.addEventListener('dragover',e=>{if(zone(e)&&files(e))e.preventDefault()});
document.addEventListener('drop',e=>{const z=zone(e);if(!z||!files(e))return;e.preventDefault();calm(z);const i=z.querySelector('.dropzone-input'),t=new DataTransfer(),
dropped=[...e.dataTransfer.files],ok=dropped.filter(f=>takes(i,f));if(!ok.length){say(z,dropped.map(f=>f.name).join(', ')+': not taken, '+i.accept+' only');return}
(i.multiple?[...i.files,...ok]:ok.slice(0,1)).forEach(f=>t.items.add(f));i.files=t.files;i.dispatchEvent(new Event('change',{bubbles:true}))})})()"#;

/// Lucide's upload, FileUpload's mark.
const UPLOAD: &str = "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12";
