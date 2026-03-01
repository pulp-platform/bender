package common_pkg;

    typedef enum logic [1:0] {
        Idle = 2'b00,
        Busy = 2'b01,
        Error = 2'b11
    } state_t;

    function automatic logic is_error(state_t s);
        return s == Error;
    endfunction

endpackage
