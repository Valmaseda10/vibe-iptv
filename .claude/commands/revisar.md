---
description: Revisión crítica del código recién escrito
---
Revisa lo que acabas de escribir contra estos criterios y sé duro contigo mismo:

- ¿Alguna ruta de código puede abrir un segundo stream simultáneo? (R1)
- ¿Algún log, error o nombre de fichero puede contener credenciales? (R5)
- ¿Alguna consulta sin índice, lista sin virtualizar o carga completa en memoria? (R4)
- ¿Algún unwrap o expect fuera de tests e inicialización?
- ¿Algún `any` en TypeScript?
- ¿Algún proceso hijo que pueda quedar huérfano si el usuario cierra la ventana?
- ¿Lógica de negocio que se te ha colado en React?

Lista lo que falla y arréglalo. Si no falla nada, dilo sin adornos.
