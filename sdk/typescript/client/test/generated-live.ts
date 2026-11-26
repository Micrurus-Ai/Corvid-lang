// Generate ../target/boards with "corvid contract ts-client boards.cor".
import type { Api } from "../target/boards/api";

export function mountCards(api: Api, display: (titles: readonly string[]) => void) {
  return api.live.board_app.cards(7, {
    rows(cards) {
      display(cards.map(card => card.title));
      // @ts-expect-error Card has no caption field.
      cards.map(card => card.caption);
    },
    error() { display([]); },
  });
}

function checkArguments(api: Api) {
  // @ts-expect-error board is an Int path parameter.
  api.live.board_app.cards("7", { rows() {}, error() {} });
  // @ts-expect-error A live subscription needs its typed observer.
  api.live.board_app.cards(7);
}
