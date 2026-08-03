module core #(
    parameter common_pkg::state_t DefaultState = common_pkg::Idle
) ();
    // Scoped type name carrying a packed dimension that is itself a scoped name.
    common_pkg::state_t [common_pkg::NumStates-1:0] state_history;

    leaf u_leaf();
endmodule
