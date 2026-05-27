import { PrismaClient } from "@prisma/client";
import { Pool } from "pg";
import { PrismaPg } from "@prisma/adapter-pg";
import dotenv from "dotenv";

dotenv.config();

const connectionString = process.env.DATABASE_URL;

// Configure resilient connection pool to survive high concurrency and prevent socket/memory leaks
export const pool = new Pool({
  connectionString,
  max: 20,                          // Keep connection pool limits stable under concurrent loads
  idleTimeoutMillis: 30000,        // Close idle connections to release resources
  connectionTimeoutMillis: 2000,   // Fail-fast on connection bottleneck (avoid hanging sockets)
});

const adapter = new PrismaPg(pool);

const globalForPrisma = global as unknown as { prisma: PrismaClient };

export const prisma =
  globalForPrisma.prisma ||
  new PrismaClient({
    adapter,
    log: process.env.NODE_ENV === "development" ? ["query", "error", "warn"] : ["error"],
  });

if (process.env.NODE_ENV !== "production") globalForPrisma.prisma = prisma;
