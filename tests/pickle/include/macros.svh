// Simple macro to test if includes are resolved correctly
`define LOG(msg) \
    $display("[LOG]: %s", msg);

// Macro that references a package symbol; pickle renaming should update this.
`define PKG_IS_ERROR(sig) \
    common_pkg::is_error(sig)

// A constant used in the RTL
localparam int unsigned DataWidth = 32;
