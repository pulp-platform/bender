`include "macros.svh"

import common_pkg::*;

module top (
    input logic clk,
    input logic rst_n
);

    core u_core();

    // Interface Instantiation
    bus_intf #(.WIDTH(DATA_WIDTH)) axi_bus (
        .clk(clk)
    );

    // Virtual Interface Type
    virtual bus_intf v_if_handle;

    initial begin
        v_if_handle = axi_bus;

`ifdef ENABLE_LOGGING
        `LOG("TopModule started successfully!")
`endif
    end

    // Type Usage from Package (state_t)
    common_pkg::state_t current_state;
    logic macro_error;

    // Undefined dependency references must not be renamed.
    undefined_pkg::undefined_t ext_state;
    undefined_mod u_ext_mod();
    virtual undefined_intf ext_if;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            current_state <= Idle;
            macro_error <= 1'b0;
        end else begin
            current_state <= Busy;
            macro_error <= `PKG_IS_ERROR(current_state);
        end
    end

endmodule
