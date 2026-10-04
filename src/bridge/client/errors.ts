/**
 * Bridge error taxonomy — verbatim reuse of pixiWeb's classes so every
 * downstream handler (`PIXICLIENT_UNAVAILABLE` → dialog, jikan bridge mode,
 * `lib/errors` mapping) keeps working unmodified.
 */
export {
  BRIDGE_ERROR_CODES,
  BridgeError,
  BridgeOfflineError,
  classifyBridgeResponse,
  isBridgeErrorCode,
  type BridgeErrorCode,
} from '../../../../pixiWeb/src/api/pixiclient/client/errors.ts'
