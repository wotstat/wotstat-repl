import { repl } from "@/shared/repl";
import type { ServerEvent } from "@/shared/api";
import { useSession } from "@/entities/session";
import { consoleBus } from "@/entities/console";

let connectGeneration = 0;

function waitForConnected(generation: number): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    let unsub: () => void = () => undefined;

    const finish = (connected: boolean) => {
      if (settled) return;
      settled = true;
      unsub();
      resolve(connected);
    };

    unsub = useSession.subscribe((state) => {
      if (state.status === "connected") finish(true);
      else if (
        generation !== connectGeneration ||
        state.status === "disconnected"
      )
        finish(false);
    });

    const status = useSession.getState().status;
    if (status === "connected") finish(true);
    else if (generation !== connectGeneration || status === "disconnected")
      finish(false);
  });
}

export async function connect(): Promise<void> {
  const generation = ++connectGeneration;
  const session = useSession.getState();
  session.setStatus("connecting");

  const onEvent = (event: ServerEvent) => {
    if (generation !== connectGeneration) return;
    if (event.kind === "log") {
      consoleBus.append(event.lines);
    } else if (event.kind === "hello") {
      const s = useSession.getState();
      s.setStatus("connected");
      s.setHello(event);
      consoleBus.system(
        `agent online (pid ${event.pid ?? "?"}, v${event.version ?? "?"})\n`,
      );
    } else if (event.kind === "disconnected") {
      const s = useSession.getState();
      if (s.status === "connected") {
        s.setStatus("disconnected");
        consoleBus.system("game disconnected\n");
      }
    }
  };

  try {
    const connection = await repl.connect(onEvent);
    if (generation !== connectGeneration) return;
    session.setEndpoint(connection.endpoint);
    if (generation !== connectGeneration) {
      await repl.disconnect().catch(() => undefined);
      return;
    }
    if (connection.waitingForAgent) {
      consoleBus.system(`listening on ${connection.endpoint} — waiting for the game\n`);
    }
    await waitForConnected(generation);
  } catch (error) {
    if (generation !== connectGeneration) return;
    useSession.getState().setStatus("disconnected");
    consoleBus.system(`connect failed: ${String(error)}\n`);
  }
}

export async function disconnect(): Promise<void> {
  connectGeneration += 1;
  await repl.disconnect().catch(() => undefined);
  useSession.getState().setStatus("disconnected");
  consoleBus.system("disconnected\n");
}
