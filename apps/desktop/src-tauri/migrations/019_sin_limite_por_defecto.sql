-- Deshace lo que hizo la primera versión de la migración 017: ponía límite de 12 años a
-- todo perfil con `adult_ok = 0`, pero eso solo significa "sin categorías XXX" y es lo
-- que el editor deja por defecto, así que limitaba perfiles de adultos (el del propio
-- usuario lo estaba). Nadie había podido elegir todavía un límite a mano: el editor que
-- lo permite llegó en la misma versión.
UPDATE profiles SET max_age = NULL WHERE max_age = 12;
