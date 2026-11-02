use sol_trade_router_sdk::{BuyWith, DexType, PROGRAM_ID, WSOL_MINT};

fn main() {
    println!("sol-trade-router-sdk probe");
    println!("router_program_id={PROGRAM_ID}");
    println!("wsol_mint={WSOL_MINT}");
    println!("native_buy_asset={BuyWith::Sol:?}");
    println!("supported_dexes={:?}", [
        DexType::PumpFun,
        DexType::PumpSwap,
        DexType::StonkFun,
        DexType::RaydiumCpmm,
        DexType::RaydiumAmmV4,
        DexType::MeteoraDammV2,
    ]);
}
