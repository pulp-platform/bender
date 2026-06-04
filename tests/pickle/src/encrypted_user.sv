// Plain RTL that instantiates the encrypted IP. Slang parses this file fine
// but the encrypted_ip reference has no corresponding tree (the encrypted
// file failed to parse), so the dangling-ref tolerance kicks in.
module encrypted_user ();
    encrypted_ip u_enc();
endmodule
