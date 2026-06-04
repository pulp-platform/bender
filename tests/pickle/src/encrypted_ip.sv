// Realistic-shape IEEE-1735 encrypted module: slang lexes/skips the protect
// envelope but the parser still trips on the surrounding endmodule, which is
// what we want this fixture to exercise.
module encrypted_ip ();
`pragma protect begin_protected
`pragma protect encrypt_agent = "Test"
`pragma protect data_method = "aes128-cbc"
`pragma protect encoding = ( enctype = "base64", bytes = 33 )
`pragma protect data_block
QUJDREVGRzEyMzQ1Njc4OTBhYmNkZWZnaGlqa2xtbm9wcXIK
`pragma protect end_protected
endmodule
