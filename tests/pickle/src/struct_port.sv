// Fixture for the kg integration test exercising the elab-time packed-struct
// port-width breakdown. `bus_top` is the elaboration root; `bus_consumer`
// receives a typedef'd packed struct port (`req_i`), a packed-array-of-structs
// port (`req_arr_i`), and returns one whose type nests another packed struct
// (`resp_o.nested_req`).

package bus_pkg;
    typedef struct packed {
        logic [31:0] addr;
        logic [2:0]  prot;
        logic        valid;
    } req_t;

    typedef struct packed {
        logic [7:0] status;
        req_t       nested_req;
    } resp_t;
endpackage

module bus_consumer (
    input  logic                  clk_i,
    input  bus_pkg::req_t         req_i,
    input  bus_pkg::req_t [3:0]   req_arr_i,
    output bus_pkg::resp_t        resp_o
);
endmodule

module bus_top (
    input logic clk
);
    bus_pkg::req_t        the_req;
    bus_pkg::req_t [3:0]  the_req_arr;
    bus_pkg::resp_t       the_resp;
    bus_consumer u_cons (
        .clk_i    (clk),
        .req_i    (the_req),
        .req_arr_i(the_req_arr),
        .resp_o   (the_resp)
    );
endmodule
