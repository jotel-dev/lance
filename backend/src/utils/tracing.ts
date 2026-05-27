import { Request, Response, NextFunction } from "express";
import { AsyncLocalStorage } from "node:async_hooks";
import { randomUUID } from "node:crypto";

// Define the type for our trace context
export interface TraceContext {
  requestId: string;
  userAddress?: string;
  [key: string]: any;
}

// Global AsyncLocalStorage for trace correlation
export const traceStorage = new AsyncLocalStorage<TraceContext>();

// Helper to get current trace context
export function getTraceContext(): TraceContext | undefined {
  return traceStorage.getStore();
}

// Structured Logging Framework
export const logger = {
  formatMessage(level: string, message: string, meta?: any): string {
    const context = getTraceContext();
    const timestamp = new Date().toISOString();
    const logData = {
      timestamp,
      level,
      message,
      requestId: context?.requestId,
      userAddress: context?.userAddress,
      ...context,
      ...meta,
    };

    // Remove duplicates or circular objects if any
    delete logData.jobs; // prevent deep serialization of DB objects

    if (process.env.NODE_ENV === "production") {
      return JSON.stringify(logData);
    } else {
      const colorMap: Record<string, string> = {
        DEBUG: "\x1b[36m", // Cyan
        INFO: "\x1b[32m",  // Green
        WARN: "\x1b[33m",  // Yellow
        ERROR: "\x1b[31m", // Red
      };
      const reset = "\x1b[0m";
      const color = colorMap[level] || reset;
      const reqIdStr = context?.requestId ? ` [reqId:${context.requestId.slice(0, 8)}]` : "";
      const metaStr = meta && Object.keys(meta).length > 0 ? ` | meta: ${JSON.stringify(meta)}` : "";
      return `${color}[${timestamp}] [${level}]${reqIdStr}: ${message}${metaStr}${reset}`;
    }
  },

  debug(message: string, meta?: any) {
    if (process.env.NODE_ENV !== "production" || process.env.LOG_LEVEL === "debug") {
      console.log(this.formatMessage("DEBUG", message, meta));
    }
  },

  info(message: string, meta?: any) {
    console.log(this.formatMessage("INFO", message, meta));
  },

  warn(message: string, meta?: any) {
    console.warn(this.formatMessage("WARN", message, meta));
  },

  error(message: string, meta?: any) {
    console.error(this.formatMessage("ERROR", message, meta));
  },
};

// Express Tracing Middleware
export function tracingMiddleware(req: Request, res: Response, next: NextFunction) {
  const requestId = (req.headers["x-request-id"] as string) || randomUUID();
  const userAddress = (req.headers["x-wallet-address"] as string) || undefined;

  res.setHeader("x-request-id", requestId);

  const context: TraceContext = {
    requestId,
    userAddress,
    method: req.method,
    url: req.originalUrl,
  };

  traceStorage.run(context, () => {
    const startTime = process.hrtime();

    logger.info(`Incoming Request: ${req.method} ${req.originalUrl}`, {
      ip: req.ip,
      userAgent: req.headers["user-agent"],
    });

    // Capture response completion to log latency
    res.on("finish", () => {
      const duration = process.hrtime(startTime);
      const durationMs = (duration[0] * 1000 + duration[1] / 1000000).toFixed(2);

      logger.info(`Request Completed: ${req.method} ${req.originalUrl} - Status ${res.statusCode} in ${durationMs}ms`, {
        statusCode: res.statusCode,
        durationMs: parseFloat(durationMs),
      });
    });

    next();
  });
}
