// Copyright (c) 2017 ETH Zurich
// Fabian Schuiki <fschuiki@iis.ee.ethz.ch>

fn main() {
    if let Err(e) = bender::cli::main() {
        let report = miette::Report::new(e);
        bender::errorln!("{report:?}");
        std::process::exit(1);
    }
}
