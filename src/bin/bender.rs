// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

fn main() {
    if let Err(e) = bender::cli::main() {
        bender::errorln!("{}", e);
        std::process::exit(1);
    }
}
