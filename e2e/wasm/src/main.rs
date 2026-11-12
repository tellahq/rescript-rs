use rescript_rs::spice::Module;
use wasm_bridge_e2e::*;

fn main() {
    print!(
        "{}",
        Module::new("Geometry")
            .with::<Size>()
            .with::<Fit>()
            .with::<Metadata>()
            .with::<Shape>()
            .with::<Format>()
            .function("./wasm/wasm_bridge_e2e.js", fitted_rescript())
            .function("./wasm/wasm_bridge_e2e.js", unit_size_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_shape_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_optional_rescript())
            .function("./wasm/wasm_bridge_e2e.js", square_root_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_format_rescript())
            .render()
    );
}
