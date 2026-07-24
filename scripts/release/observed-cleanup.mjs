// Release automation retains the primary failure when resource cleanup also fails.

export async function withObservedCleanup(action, cleanup, combinedMessage) {
  let outcome;
  try {
    outcome = { status: "fulfilled", value: await action() };
  } catch (error) {
    outcome = { reason: error, status: "rejected" };
  }

  try {
    await cleanup();
  } catch (cleanupError) {
    if (outcome.status === "fulfilled") {
      throw cleanupError;
    }
    throw new AggregateError([outcome.reason, cleanupError], combinedMessage);
  }
  if (outcome.status === "rejected") {
    throw outcome.reason;
  }
  return outcome.value;
}
