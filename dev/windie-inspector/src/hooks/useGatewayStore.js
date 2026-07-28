import { useCallback, useEffect, useState } from "react";
import { apiRequest } from "@/lib/windieApi";

/** Owns the browser projection of backend gateway availability. */
export function useGatewayStore({ onError }) {
  const [gatewayRunning, setGatewayRunning] = useState(false);

  const refreshGateway = useCallback(async () => {
    const body = await apiRequest("/api/status");
    setGatewayRunning(Boolean(body.gateway_running));
    return body.gateway_running;
  }, []);

  useEffect(() => {
    refreshGateway().catch(onError);
  }, [onError, refreshGateway]);

  return { gatewayRunning, refreshGateway };
}
