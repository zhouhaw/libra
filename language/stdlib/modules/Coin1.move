address 0x1 {

module Coin1 {
    use 0x1::Libra;
    use 0x1::FixedPoint32;

    struct Coin1 { }

    public fun initialize(
        lr_account: &signer,
        tc_account: &signer,
    ) {
        Libra::register_SCS_currency<Coin1>(
            lr_account,
            tc_account,
            FixedPoint32::create_from_rational(1, 2), // exchange rate to LBR
            1000000, // scaling_factor = 10^6
            100,     // fractional_part = 10^2
            b"Coin1"
        );
    }
}
}
