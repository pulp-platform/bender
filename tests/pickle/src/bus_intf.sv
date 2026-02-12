interface bus_intf #(
    parameter int Width = 32
) (
    input logic clk
);
    logic [Width-1:0] addr;
    logic [Width-1:0] data;
    logic             valid;
    logic             ready;

    modport master (
        output addr, data, valid,
        input  ready
    );

    modport slave (
        input  addr, data, valid,
        output ready
    );

endinterface
