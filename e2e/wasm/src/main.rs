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
            .function("./wasm/wasm_bridge_e2e.js", label_or_rescript())
            .function("./wasm/wasm_bridge_e2e.js", square_root_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_format_rescript())
            .with::<Curve>()
            .function("./wasm/wasm_bridge_e2e.js", echo_curve_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_number_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_count_rescript())
            .function("./wasm/wasm_bridge_e2e.js", echo_flag_rescript())
            .class("./wasm/wasm_bridge_e2e.js", tally_rescript())
            .with::<Reading>()
            .function("./wasm/wasm_bridge_e2e.js", readings_rescript())
            .with::<Stored>()
            .function("./wasm/wasm_bridge_e2e.js", find_stored_rescript())
            .with::<Positions>()
            .function("./wasm/wasm_bridge_e2e.js", positions_rescript())
            .render()
    );
}
