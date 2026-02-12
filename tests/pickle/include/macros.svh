// Simple macro to test if includes are resolved correctly
`define LOG(msg) \
    $display("[LOG]: %s", msg);

// A constant used in the RTL
localparam int unsigned DataWidth = 32;
