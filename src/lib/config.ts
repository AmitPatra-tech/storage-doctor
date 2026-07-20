// Dodo Payments configuration.
//
// Dodo's license endpoints (validate / activate / deactivate) are PUBLIC — they
// do not require the secret API key — so the app validates license keys
// directly against Dodo. No backend and no secret in the client are needed.
//
// `dodoMode` selects the Dodo environment:
//  - "live" — real payments and real license keys.
//  - "test" — Dodo test environment (test cards, test license keys). Use this
//    while setting things up so you don't charge yourself.
export const CONFIG = {
  dodoMode: "live" as "test" | "live",
  dodoLiveProductId: "pdt_0NjSdjhwfCyngGGbuPzsN",
  dodoTestProductId: "pdt_0NjU7StVxETIrbHUVx8A0",
  proPriceLabel: "₹59 one-time",
  invoiceEmail: "mikarmiaura@gmail.com",
};

const LIVE = {
  api: "https://live.dodopayments.com",
  checkout: "https://checkout.dodopayments.com",
};
const TEST = {
  api: "https://test.dodopayments.com",
  checkout: "https://test.checkout.dodopayments.com",
};

const env = CONFIG.dodoMode === "test" ? TEST : LIVE;
const productId = CONFIG.dodoMode === "test" ? CONFIG.dodoTestProductId : CONFIG.dodoLiveProductId;

/** Base URL for Dodo's public license API. */
export const dodoApiBase = env.api;

/** Hosted checkout link for the Pro product. */
export const dodoCheckoutUrl = `${env.checkout}/buy/${productId}`;
