mod api;
mod app;
mod auth;
mod components;
mod models;
mod storage;
mod url;
mod ws;

use app::App;

fn main() {
    yew::Renderer::<App>::new().render();
}
