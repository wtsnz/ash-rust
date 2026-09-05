import type { APIRoute } from 'astro';
import { ash, OpenTicketInputSchema } from '../../lib/client';

export const GET: APIRoute = async () => {
  try {
    const tickets = await ash.ticket
      .query()
      .include({ author: true })
      .sort("priority", "asc")
      .all();

    return new Response(JSON.stringify({ success: true, data: tickets }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  } catch (err: any) {
    return new Response(
      JSON.stringify({ success: false, error: err.message }),
      {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      }
    );
  }
};

export const POST: APIRoute = async ({ request }) => {
  try {
    const body = await request.json();

    // Validate with ash-typescript Zod schema
    const parseResult = OpenTicketInputSchema.safeParse(body);
    if (!parseResult.success) {
      return new Response(
        JSON.stringify({
          success: false,
          errors: parseResult.error.flatten(),
        }),
        { status: 422, headers: { 'Content-Type': 'application/json' } }
      );
    }

    const ticket = await ash.ticket.open(parseResult.data, { author: true });

    return new Response(JSON.stringify({ success: true, data: ticket }), {
      status: 201,
      headers: { 'Content-Type': 'application/json' },
    });
  } catch (err: any) {
    return new Response(
      JSON.stringify({ success: false, error: err.message }),
      {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      }
    );
  }
};
