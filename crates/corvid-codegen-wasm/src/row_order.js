function placeOrderedRows(list, ordered) {
  // Build both sides around the focused row so it never leaves the document.
  const focused = list.getRootNode().activeElement;
  const pivot = ordered.findIndex(row => row.contains(focused));
  let anchor = pivot < 0 ? null : ordered[pivot];
  for (let index = pivot - 1; index >= 0; index--) {
    const row = ordered[index];
    if (row.parentNode !== list || row.nextElementSibling !== anchor) list.insertBefore(row, anchor);
    anchor = row;
  }
  anchor = pivot < 0 ? list.firstElementChild : ordered[pivot].nextElementSibling;
  for (let index = pivot + 1; index < ordered.length; index++) {
    const row = ordered[index];
    if (row !== anchor) list.insertBefore(row, anchor);
    anchor = row.nextElementSibling;
  }
}
