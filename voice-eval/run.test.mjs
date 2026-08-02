import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

// hedgedCall bor i hedge.mjs — IKKE i run.mjs. run.mjs starter Gate 1 ved
// import, saa den maa aldrig importeres herfra (se hedge.mjs' hoved).
import { hedgedCall } from "./hedge.mjs";

// En promise vi selv afgoer tidspunktet for — bruges til at holde skud 1
// "in-flight" mens hedge-skuddet naar at fejle.
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

describe("hedgedCall", () => {
  it("lader det foerste Ok vinde og affyrer aldrig hedgen", async () => {
    let calls = 0;
    const fire = () => {
      calls += 1;
      return Promise.resolve("hurtigt");
    };

    await expect(hedgedCall(fire, 5)).resolves.toBe("hurtigt");
    // Timeren SKAL vaere ryddet: havde den overlevet, ville skud 2 blive
    // affyret her — et betalt kald ingen laeser svaret paa.
    await tick(30);
    expect(calls).toBe(1);
  });

  it("bruger hedge-skuddet naar det foerste skud fejler", async () => {
    let calls = 0;
    const fire = () => {
      calls += 1;
      return calls === 1
        ? Promise.reject(new Error("foerste skud doede"))
        : Promise.resolve("hedge-ok");
    };

    await expect(hedgedCall(fire, 1)).resolves.toBe("hedge-ok");
  });

  it("sluger ikke skud 1's sene Ok naar hedge-skuddet fejler foerst", async () => {
    // Regressionen: hedgen affyres, faar en oejeblikkelig 429, og skud 1
    // svarer foerst BAGEFTER. Talte vi ikke afsluttede forsoeg, afviste den
    // ydre promise paa hedge-fejlen og kastede netop det svar vaek som hele
    // hedgen findes for at redde.
    const slow = deferred();
    let calls = 0;
    const fire = () => {
      calls += 1;
      return calls === 1
        ? slow.promise
        : Promise.reject(new Error("429 fra hedge-skuddet"));
    };

    const result = hedgedCall(fire, 1);
    // Observatoeren haenges paa MED DET SAMME: den gamle kode afviste allerede
    // ved hedge-fejlen, og uden en handler ville det blive til en unhandled
    // rejection i stedet for en laesbar assert.
    const observed = [];
    result.then(
      (value) => observed.push(`ok:${value}`),
      (error) => observed.push(`err:${error.message}`),
    );

    await tick(30);
    expect(calls).toBe(2);
    expect(observed).toEqual([]);

    slow.resolve("sent-men-godt-svar");
    await expect(result).resolves.toBe("sent-men-godt-svar");
  });

  it("afviser foerst naar BEGGE skud er faldet, og med det primaere skuds fejl", async () => {
    const slow = deferred();
    let calls = 0;
    const fire = () => {
      calls += 1;
      return calls === 1
        ? slow.promise
        : Promise.reject(new Error("hedge-foelgefejl"));
    };

    const result = hedgedCall(fire, 1);
    const observed = [];
    result.then(
      () => observed.push("ok"),
      (error) => observed.push(error.message),
    );

    await tick(30);
    // Hedgen er faldet, men skud 1 lever — intet er afgjort endnu.
    expect(observed).toEqual([]);

    slow.reject(new Error("den oprindelige aarsag"));
    await expect(result).rejects.toThrow("den oprindelige aarsag");
  });

  it("rapporterer skud 1's fejl naar den kom foerst og begge fejler", async () => {
    let calls = 0;
    const fire = () => {
      calls += 1;
      return Promise.reject(
        new Error(calls === 1 ? "den oprindelige aarsag" : "hedge-foelgefejl"),
      );
    };

    await expect(hedgedCall(fire, 1)).rejects.toThrow("den oprindelige aarsag");
  });
});

describe("run.mjs' entry", () => {
  // Gate 1 skal koere naar filen loades — punktum. Bliver main() paa noget
  // tidspunkt gated bag en process.argv[1]-vagt (fx for at kunne importere en
  // hjaelper herfra), slukker gaten LYDLOEST under `npx vite-node`, som er den
  // maade runnerne faktisk koeres paa: dér peger argv[1] paa
  // vite-node's egen CLI, vagten er falsk, og kommandoen afslutter exit 0 med
  // nul output. Kilde-laesning frem for import: en import ville betale for 48
  // rigtige API-kald midt i suiten.
  it("kalder main() ubetinget — ingen argv-vagt foran Gate 1", async () => {
    const source = await readFile(new URL("./run.mjs", import.meta.url), "utf8");

    // Uindrykket = top-level. Round 1's vagt havde main() indrykket i et if.
    expect(source).toMatch(/^main\(\)\.catch\(/mu);
    expect(source).not.toMatch(/process\.argv\[1\]/u);
  });
});
