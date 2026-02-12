`include "macros.svh"

import common_pkg::*;

module top (
    input logic clk,
    input logic rst_n
);

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

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            current_state <= Idle;
        end else begin
            current_state <= Busy;
        end
    end

endmodule
