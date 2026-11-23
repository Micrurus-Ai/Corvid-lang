// The shared storefront controls call these handlers with validated quantities.
function instantiate() {
  return {
    notebook_cents(quantity: bigint): bigint {
      return quantity * 450n;
    },
    pencil_cents(quantity: bigint): bigint {
      return quantity * 125n;
    },
  };
}
